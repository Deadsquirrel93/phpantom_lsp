//! High-level uncaught-throws analysis with cross-file `@throws`
//! propagation.
//!
//! Ties together [`super::scanning`] and [`super::catch`] to determine
//! which exception types a function body throws but does not catch,
//! optionally resolving calls into other files via a [`ThrowsContext`]:
//!
//! - `$variable->method()` — the variable's type is resolved from the
//!   function's parameter list, the class is loaded, and the method's
//!   `@throws` tags are propagated.
//! - `ClassName::staticMethod()` — the class is loaded directly and the
//!   method's `@throws` tags are propagated.
//! - `functionName()` — the function is loaded and its `@throws` tags
//!   are propagated.
//! - `new ClassName(…)` — the class is loaded and the constructor's
//!   `@throws` tags are propagated.

use crate::atom::atom;
use std::collections::HashMap;
use std::sync::Arc;

use tower_lsp::lsp_types::Position;

use crate::completion::source::comment_position::position_to_byte_offset;
use crate::php_type::PhpType;
use crate::text_scan::{CodeOffsets, is_ident_byte, keyword_starts_at};
use crate::types::{ClassInfo, FunctionLoader};

use super::catch::{CatchInfo, find_catch_blocks, find_throw_variable_types};
use super::scanning::{
    ThrowInfo, extract_function_body, find_inline_throws_annotations, find_method_throws_tags,
    find_propagated_throws, find_throw_expression_types, find_throw_statements,
};

/// Optional reference to a class-loader closure, used by nested
/// helper functions in throws analysis.
type OptClassLoader<'a> = Option<&'a dyn Fn(&str) -> Option<Arc<ClassInfo>>>;

/// Bundles the loaders needed for cross-file throws resolution.
///
/// When provided to [`find_uncaught_throw_types_with_context`], every call
/// in the function body is inspected for cross-file `@throws` propagation
/// (see the module-level docs for the call patterns this covers).
pub(crate) struct ThrowsContext<'a> {
    /// Resolves a class name to its [`ClassInfo`].
    pub class_loader: &'a dyn Fn(&str) -> Option<Arc<ClassInfo>>,
    /// Resolves a function name to its [`FunctionInfo`].
    pub function_loader: FunctionLoader<'a>,
    /// Use-statement map for the current file (short name → FQN).
    /// Used to resolve short exception names to fully-qualified names.
    pub use_map: &'a HashMap<String, String>,
    /// The namespace of the current file, if any.
    pub file_namespace: &'a Option<String>,
}

/// Determine which exception types in a function body are **not** caught
/// by an enclosing `try/catch` block.
///
/// Detects six patterns (same-file only):
/// 1. `throw new ExceptionType(…)` (direct instantiation)
/// 2. `throw $this->method()` / `throw self::method()` / `throw static::method()`
///    (the method's return type is the thrown exception type)
/// 3. `throw functionName()` (bare function call, return type is thrown)
/// 4. `$this->method()` / `self::method()` calls where the called method's
///    docblock declares `@throws ExceptionType` (propagated throws, same file)
/// 5. Inline `/** @throws ExceptionType */` annotations in the function body
/// 6. `throw $variable` (resolved through enclosing catch clause variable)
///
/// Returns a deduplicated list of short exception type names.
///
/// This variant does **not** perform cross-file resolution.
/// Use [`find_uncaught_throw_types_with_context`] with a [`ThrowsContext`]
/// to enable it.
pub(crate) fn find_uncaught_throw_types(content: &str, position: Position) -> Vec<PhpType> {
    find_uncaught_throw_types_with_context(content, position, None)
}

/// Like [`find_uncaught_throw_types`] but with an optional [`ThrowsContext`]
/// for cross-file throws propagation.
///
/// When a context is provided, **every** call in the function body is
/// inspected for cross-file `@throws` tags (see the module-level docs for
/// the call patterns this covers).
pub(crate) fn find_uncaught_throw_types_with_context(
    content: &str,
    position: Position,
    ctx: Option<&ThrowsContext<'_>>,
) -> Vec<PhpType> {
    let body = match extract_function_body(content, position) {
        Some(b) => b,
        None => return Vec::new(),
    };

    let throws = find_throw_statements(&body);
    let throw_expr_types = find_throw_expression_types(&body, content);
    let propagated = find_propagated_throws(&body, content);
    let catches = find_catch_blocks(&body);
    let throw_vars = find_throw_variable_types(&body, &catches);

    // Cross-file propagated throws from all call patterns.
    let cross_file_propagated = if let Some(throws_ctx) = ctx {
        let signature = extract_function_signature(content, position);
        find_cross_file_propagated_throws(&body, &signature, content, throws_ctx)
    } else {
        Vec::new()
    };

    let mut uncaught: Vec<PhpType> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    /// Check whether a throw at `offset` in the function body is caught
    /// by one of the `catches`, given the exception type.
    fn is_caught_by(
        catches: &[CatchInfo],
        offset: usize,
        exc_type: &PhpType,
        use_map: &HashMap<String, String>,
        file_namespace: &Option<String>,
        class_loader: OptClassLoader<'_>,
    ) -> bool {
        // Already resolved by `normalize_throw_type`.
        let exc_fqn = exc_type.base_name().unwrap_or("");
        catches.iter().any(|c| {
            offset > c.try_start
                && offset < c.try_end
                && c.type_names.iter().any(|ct| {
                    let resolved_ct = canonical_fqn(
                        crate::names::resolve_to_fqn(&ct.to_string(), use_map, file_namespace),
                        class_loader,
                    );
                    // Check for Throwable/Exception (catches everything).
                    let resolved_lower = resolved_ct.to_ascii_lowercase();
                    if resolved_lower == "throwable" || resolved_lower == "exception" {
                        return true;
                    }
                    // Exact FQN comparison (case-insensitive)
                    resolved_ct.eq_ignore_ascii_case(exc_fqn)
                })
        })
    }

    /// Normalize a ThrowInfo's type_name into a TypeKind::Named,
    /// resolving short names to FQN when use_map/namespace are available.
    fn normalize_throw_type(
        ty: &PhpType,
        use_map: &HashMap<String, String>,
        file_namespace: &Option<String>,
        class_loader: OptClassLoader<'_>,
    ) -> Option<PhpType> {
        let raw = ty.to_string();
        if raw.trim_start_matches('\\').is_empty() {
            None
        } else {
            let resolved = crate::names::resolve_to_fqn(&raw, use_map, file_namespace);
            Some(PhpType::named(atom(&canonical_fqn(resolved, class_loader))))
        }
    }

    /// The declared spelling of the class `fqn` names, or `fqn` itself
    /// when it cannot be loaded.
    fn canonical_fqn(fqn: String, class_loader: OptClassLoader<'_>) -> String {
        class_loader
            .and_then(|loader| crate::class_lookup::load_resolved_name(&fqn, loader))
            .map(|cls| cls.fqn().to_string())
            .unwrap_or(fqn)
    }

    // Determine name-resolution context: use the ThrowsContext fields
    // when available, fall back to empty defaults.
    let empty_use_map = HashMap::new();
    let empty_ns = None;
    let (use_map, file_namespace) = if let Some(throws_ctx) = ctx {
        (throws_ctx.use_map, throws_ctx.file_namespace)
    } else {
        (&empty_use_map, &empty_ns)
    };

    let class_loader: OptClassLoader<'_> =
        ctx.map(|c| c.class_loader as &dyn Fn(&str) -> Option<Arc<ClassInfo>>);

    // In order: direct `throw new Type(…)` statements, `throw
    // $this->method()` (the method's return type is the thrown type),
    // `@throws` propagated from same-file calls, inline `/** @throws Type */`
    // annotations, `throw $variable` resolved from the catch clause's type,
    // and `@throws` propagated from calls into other files.
    let inline = find_inline_throws_annotations(&body);
    let thrown = throws
        .iter()
        .chain(&throw_expr_types)
        .chain(&propagated)
        .chain(&inline)
        .chain(&throw_vars)
        .chain(&cross_file_propagated);
    for throw in thrown {
        if let Some(exc_type) =
            normalize_throw_type(&throw.type_name, use_map, file_namespace, class_loader)
            && !is_caught_by(
                &catches,
                throw.offset,
                &exc_type,
                use_map,
                file_namespace,
                class_loader,
            )
            && seen.insert(exc_type.to_string())
        {
            uncaught.push(exc_type);
        }
    }

    uncaught
}

/// Extract the function/method signature (the text between `function` and `{`)
/// from the content at the given position.
///
/// Returns the raw signature string, e.g.
/// `"handle(BusinessCentralService $service): void"`.
pub(crate) fn extract_function_signature(content: &str, position: Position) -> String {
    let byte_offset = position_to_byte_offset(content, position);
    let after_cursor = &content[byte_offset.min(content.len())..];

    let after_docblock = if let Some(close_pos) = after_cursor.find("*/") {
        &after_cursor[close_pos + 2..]
    } else {
        after_cursor
    };

    // Find the `function` keyword.
    let lower = after_docblock.to_lowercase();
    let func_pos = match lower.find("function") {
        Some(p) => p,
        None => return String::new(),
    };

    let after_func = &after_docblock[func_pos + 8..]; // skip "function"

    // Everything up to the opening brace is the signature.
    match after_func.find('{') {
        Some(brace) => after_func[..brace].to_string(),
        None => String::new(),
    }
}

/// Parse a function signature to build a map of `$variable_name -> TypeName`.
///
/// Given a signature like `"handle(BusinessCentralService $service, int $count): void"`,
/// returns `[("$service", "BusinessCentralService"), ("$count", "int")]`.
pub(crate) fn parse_param_type_map(signature: &str) -> Vec<(String, PhpType)> {
    let mut result = Vec::new();

    // Extract the text inside the outermost parentheses.
    let open = match signature.find('(') {
        Some(p) => p,
        None => return result,
    };
    let close = match signature.rfind(')') {
        Some(p) => p,
        None => return result,
    };
    if close <= open {
        return result;
    }

    let params_text = &signature[open + 1..close];

    // Split on commas, respecting nested parentheses/generics.
    let mut depth = 0i32;
    let mut start = 0;
    let bytes = params_text.as_bytes();
    let mut segments = Vec::new();

    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'<' | b'[' | b'{' => depth += 1,
            b')' | b'>' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                segments.push(&params_text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    segments.push(&params_text[start..]);

    for segment in segments {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Each parameter segment looks like:
        //   [?]TypeName [&][$]varName [= default]
        // We need to find the first `$name` token and the type before it.
        // Skip promoted property modifiers (public/protected/private/readonly).
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();

        // Find the variable name (starts with `$`, possibly prefixed with `&` or `...`).
        let var_idx = tokens.iter().position(|t| {
            let t = t.trim_start_matches('&').trim_start_matches("...");
            t.starts_with('$')
        });

        let var_idx = match var_idx {
            Some(i) => i,
            None => continue,
        };

        if var_idx == 0 {
            // No type before the variable name.
            continue;
        }

        let var_name = tokens[var_idx]
            .trim_start_matches('&')
            .trim_start_matches("...");

        // The type is immediately before the variable. Skip modifiers.
        let type_idx = var_idx - 1;
        let type_token = tokens[type_idx];

        // Skip PHP modifiers that aren't types.
        if matches!(
            type_token.to_lowercase().as_str(),
            "public" | "protected" | "private" | "readonly"
        ) {
            continue;
        }

        // Strip the nullable wrapper, keeping a class name as written (a
        // leading `\` included) for the caller to resolve.
        let parsed = PhpType::parse(type_token);
        let non_null = parsed.non_null_type().unwrap_or_else(|| parsed.clone());
        if non_null.base_name().is_some() {
            result.push((var_name.to_string(), non_null));
        } else {
            // Fallback for scalars and other non-class types.
            let cleaned_type = type_token.trim_start_matches('?').trim_start_matches('\\');
            if !cleaned_type.is_empty() {
                result.push((var_name.to_string(), PhpType::named(atom(cleaned_type))));
            }
        }
    }

    result
}

/// Find `@throws` annotations from all call patterns in the function body
/// by resolving types via the class and function loaders.
///
/// Handles:
/// - `$variable->method()` — resolves variable type from function params
/// - `ClassName::staticMethod()` — loads the class directly
/// - `functionName()` — loads the function directly
/// - `new ClassName(…)` — loads the class and checks the constructor
pub(crate) fn find_cross_file_propagated_throws(
    body: &str,
    signature: &str,
    file_content: &str,
    ctx: &ThrowsContext<'_>,
) -> Vec<ThrowInfo> {
    let raw_param_map = parse_param_type_map(signature);
    let class_loader = ctx.class_loader;

    // Resolve short param type names to FQN so that class_loader lookups
    // succeed even when the signature uses unqualified class names.
    let param_map: Vec<(String, PhpType)> = raw_param_map
        .into_iter()
        .map(|(name, ty)| {
            // Resolve before taking the base name, which drops a leading `\`.
            let resolved = ty
                .resolve_names(&|name| {
                    crate::names::resolve_to_fqn(name, ctx.use_map, ctx.file_namespace)
                })
                .base_name()
                .map_or(ty, |base| PhpType::named(atom(base)));
            (name, resolved)
        })
        .collect();

    let mut results = Vec::new();
    let mut seen_calls = std::collections::HashSet::new();

    let bytes = body.as_bytes();
    let len = bytes.len();
    let mut offsets = CodeOffsets::new(bytes);

    while let Some(mut pos) = offsets.next() {
        // ── Pattern: `new ClassName(…)` ─────────────────────────────
        if keyword_starts_at(bytes, pos, b"new") {
            let call_start = pos;
            let after_new = body[pos + 3..].trim_start();
            // Extract class name (may be namespaced with `\`).
            let name_end = after_new
                .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '\\')
                .unwrap_or(after_new.len());
            let class_name = &after_new[..name_end];
            let after_name = after_new[name_end..].trim_start();
            if !class_name.is_empty() && after_name.starts_with('(') {
                let resolved_class =
                    crate::names::resolve_to_fqn(class_name, ctx.use_map, ctx.file_namespace);
                let call_key = format!("new:{}", resolved_class);
                if seen_calls.insert(call_key)
                    && let Some(class_info) =
                        crate::class_lookup::load_resolved_name(&resolved_class, class_loader)
                    && let Some(ctor) = class_info.get_method("__construct")
                {
                    for exc_type in ctor.throws() {
                        results.push(ThrowInfo {
                            type_name: as_written_fqn(exc_type),
                            offset: call_start,
                        });
                    }
                }
            }
            offsets.skip_to(pos + 3);
            continue;
        }

        // ── Pattern: `$variable->method()` ──────────────────────────
        if bytes[pos] == b'$' {
            let var_start = pos;
            pos += 1;
            // Collect variable name characters.
            while pos < len && is_ident_byte(bytes[pos]) {
                pos += 1;
            }
            offsets.skip_to(pos);
            let var_name = &body[var_start..pos];

            // Check for `->` immediately after (whitespace-tolerant).
            let rest = &body[pos..];
            let trimmed = rest.trim_start();
            if !trimmed.starts_with("->") {
                continue;
            }
            let arrow_offset = rest.len() - trimmed.len() + 2; // skip "->"
            let after_arrow = &rest[arrow_offset..];
            let after_arrow_trimmed = after_arrow.trim_start();

            // Extract method name.
            let name_end = after_arrow_trimmed
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(after_arrow_trimmed.len());
            let method_name = &after_arrow_trimmed[..name_end];

            if method_name.is_empty() {
                continue;
            }

            // Check that it's followed by `(` (a method call, not a property).
            let after_method = after_arrow_trimmed[name_end..].trim_start();
            if !after_method.starts_with('(') {
                continue;
            }

            // Skip `$this` — those are handled by find_propagated_throws.
            if var_name == "$this" {
                continue;
            }

            // De-duplicate: only process each (variable, method) pair once.
            let call_key = format!("{}::{}", var_name, method_name);
            if !seen_calls.insert(call_key) {
                continue;
            }

            // Look up the variable's type from the parameter map.
            let class_name = match param_map.iter().find(|(name, _)| name == var_name) {
                Some((_, type_name)) => type_name.base_name().unwrap_or(""),
                None => continue,
            };

            // Load the class and find the method's @throws tags.
            collect_method_throws(
                class_loader,
                class_name,
                method_name,
                var_start,
                &mut results,
            );

            continue;
        }

        // ── Pattern: identifier — could be `ClassName::method()` or `functionName()` ──
        if bytes[pos].is_ascii_alphabetic() || bytes[pos] == b'_' || bytes[pos] == b'\\' {
            let ident_start = pos;
            // Collect identifier characters (including namespace separators).
            while pos < len
                && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_' || bytes[pos] == b'\\')
            {
                pos += 1;
            }
            offsets.skip_to(pos);
            let ident = &body[ident_start..pos];

            let after_ident = body[pos..].trim_start();

            // ── Sub-pattern: `ClassName::method()` ──────────────────
            if let Some(after_colons_raw) = after_ident.strip_prefix("::") {
                let after_colons = after_colons_raw.trim_start();
                let method_end = after_colons
                    .find(|c: char| !c.is_alphanumeric() && c != '_')
                    .unwrap_or(after_colons.len());
                let method_name = &after_colons[..method_end];
                let after_method = after_colons[method_end..].trim_start();

                if !method_name.is_empty() && after_method.starts_with('(') {
                    // Skip self::/static::/parent:: — handled by same-file propagation.
                    let ident_lower = ident.to_lowercase();
                    if ident_lower != "self" && ident_lower != "static" && ident_lower != "parent" {
                        let resolved_class =
                            crate::names::resolve_to_fqn(ident, ctx.use_map, ctx.file_namespace);
                        let call_key = format!("{}::{}", resolved_class, method_name);
                        if seen_calls.insert(call_key) {
                            collect_method_throws(
                                class_loader,
                                &resolved_class,
                                method_name,
                                ident_start,
                                &mut results,
                            );
                        }
                    }
                }
                continue;
            }

            // ── Sub-pattern: `functionName()` ───────────────────────
            if after_ident.starts_with('(') {
                // Skip PHP keywords that look like function calls.
                let ident_lower = ident.to_lowercase();
                if !is_php_keyword(&ident_lower) {
                    let clean_name = ident.trim_start_matches('\\');
                    let call_key = format!("fn:{}", clean_name);
                    if seen_calls.insert(call_key) {
                        // First check same-file methods (already handled by
                        // find_propagated_throws for $this->/self::/static::).
                        // For standalone functions, use the function loader.
                        if let Some(func_loader) = ctx.function_loader
                            && let Some(func_info) = func_loader(clean_name, 0)
                        {
                            for exc_type in &func_info.throws {
                                results.push(ThrowInfo {
                                    type_name: as_written_fqn(exc_type),
                                    offset: ident_start,
                                });
                            }
                        }
                        // Also check: it might be a same-file function with @throws
                        // in its docblock (text-search fallback).
                        let same_file_throws = find_method_throws_tags(file_content, clean_name);
                        for t in same_file_throws {
                            results.push(ThrowInfo {
                                type_name: t,
                                offset: ident_start,
                            });
                        }
                    }
                }
            }
        }
    }

    results
}

/// Load a class by name and collect its method's `@throws` into `results`.
pub(crate) fn collect_method_throws(
    class_loader: &dyn Fn(&str) -> Option<Arc<ClassInfo>>,
    class_name: &str,
    method_name: &str,
    offset: usize,
    results: &mut Vec<ThrowInfo>,
) {
    if let Some(class_info) = crate::class_lookup::load_resolved_name(class_name, class_loader)
        && let Some(method_info) = class_info.get_method(method_name)
    {
        for exc_type in method_info.throws() {
            results.push(ThrowInfo {
                type_name: as_written_fqn(exc_type),
                offset,
            });
        }
    }
}

/// A declaration's already-resolved `@throws` type, spelled with a leading
/// `\` so that reading it as source text in the caller's file
/// ([`find_uncaught_throw_types_with_context`]) keeps it exact.
fn as_written_fqn(ty: &PhpType) -> PhpType {
    ty.resolve_names(&|name| format!("\\{}", name.trim_start_matches('\\')))
}

/// Check whether an identifier is a PHP keyword that should not be
/// treated as a function call (e.g. `if(…)`, `foreach(…)`, `return`).
pub(crate) fn is_php_keyword(ident: &str) -> bool {
    matches!(
        ident,
        "if" | "else"
            | "elseif"
            | "while"
            | "for"
            | "foreach"
            | "switch"
            | "match"
            | "return"
            | "echo"
            | "print"
            | "isset"
            | "unset"
            | "empty"
            | "list"
            | "array"
            | "die"
            | "exit"
            | "eval"
            | "catch"
            | "throw"
            | "yield"
            | "clone"
            | "include"
            | "include_once"
            | "require"
            | "require_once"
            | "new"
            | "self"
            | "static"
            | "parent"
            | "fn"
            | "function"
            | "class"
    )
}
