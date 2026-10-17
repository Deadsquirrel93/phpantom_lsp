//! PHP parsing and AST extraction.
//!
//! This module contains the logic for parsing PHP source text using the
//! mago_syntax parser and extracting class information (methods, properties,
//! constants), `use` statement mappings, and namespace declarations from
//! the resulting AST.
//!
//! Sub-modules:
//! - [`classes`]: Class, interface, trait, and enum extraction
//! - [`anonymous`]: Anonymous class (`new class { ... }`) extraction
//! - [`attributes`]: Native PHP `#[Attribute]` target bitmask extraction
//! - [`stub_attributes`]: phpstorm-stubs version, type and shape attributes,
//!   and `#[Deprecated]`
//! - [`functions`]: Standalone function and `define()` constant extraction
//! - [`signature`]: Signature extraction shared by functions and methods
//! - [`use_statements`]: `use` statement and namespace extraction
//! - [`ast_update`]: The `update_ast` orchestrator
//! - [`ast_publish`]: Publishing a batch of parsed files to the indexes
//! - [`name_resolution`]: Qualifying the names a parsed declaration spells
//! - [`initializers`]: Qualifying the classes a constant initializer names
//! - [`parse_cache`]: The thread-local cache that shares one parse per pass
//! - [`ast_helpers`]: Small helpers for walking the AST
//! - [`error_format`]: Human-readable formatting for Mago parse errors

use std::collections::HashMap;

mod anonymous;
mod ast_helpers;
mod ast_publish;
mod ast_update;
mod attributes;
mod classes;
pub(crate) mod error_format;
mod functions;
mod initializers;
mod name_resolution;
mod parse_cache;
mod signature;
mod stub_attributes;
mod use_statements;

pub(crate) use ast_helpers::*;
pub(crate) use parse_cache::*;
pub(crate) use stub_attributes::*;

use crate::atom::{atom, bytes_to_str};

use mago_span::HasSpan;
use mago_syntax::cst::*;

use crate::php_type::PhpType;
use crate::types::*;

/// Context for resolving PHPDoc type annotations from docblock comments.
///
/// Bundles the program's trivia (comments/whitespace) and the raw source
/// text so that extraction functions can look up the `/** ... */` comment
/// preceding any AST node and parse `@return` / `@var` tags from it.
pub(crate) struct DocblockCtx<'a> {
    pub trivias: &'a [Trivia<'a>],
    pub content: &'a str,
    /// Target PHP version for version-aware stub filtering.
    ///
    /// When `Some`, elements annotated with
    /// `#[PhpStormStubsElementAvailable]` whose version range excludes
    /// this version are filtered out during extraction.  Set when
    /// parsing phpstorm-stubs; left as `None` for user code (where the
    /// attribute is never used).
    pub php_version: Option<PhpVersion>,
    /// Use-statement map for the file being parsed.
    ///
    /// Maps short (imported or aliased) names to their fully-qualified
    /// equivalents, e.g. `"Available"` → `"JetBrains\PhpStorm\Internal\PhpStormStubsElementAvailable"`.
    /// Used to resolve attribute names that appear under an alias.
    pub use_map: HashMap<String, String>,
    /// The file-level namespace, if any.
    ///
    /// Used by `is_known_deprecated_attr` to distinguish unqualified
    /// `#[Deprecated]` in the global namespace (which is the native PHP
    /// 8.4 attribute) from `#[Deprecated]` inside a user namespace (which
    /// would resolve to `App\Deprecated`, not the built-in).
    pub namespace: Option<String>,
}

/// Convert a Mago [`Hint`] AST node directly to a [`PhpType`].
///
/// Since `Hint` only represents native PHP type syntax (no
/// generics, shapes, or callables), the mapping is straightforward.
pub(crate) fn extract_hint_type(hint: &Hint) -> PhpType {
    match hint {
        Hint::Identifier(ident) => PhpType::named(atom(bytes_to_str(ident.value()))),
        Hint::Nullable(nullable) => PhpType::nullable(extract_hint_type(nullable.hint)),
        Hint::Union(union) => {
            let mut members = Vec::new();
            collect_union_members(union.left, &mut members);
            collect_union_members(union.right, &mut members);
            PhpType::union(members)
        }
        Hint::Intersection(intersection) => {
            let mut members = Vec::new();
            collect_intersection_members(intersection.left, &mut members);
            collect_intersection_members(intersection.right, &mut members);
            PhpType::intersection(members)
        }
        Hint::Null(_) => PhpType::null(),
        Hint::True(_) => PhpType::true_(),
        Hint::False(_) => PhpType::false_(),
        Hint::Array(_) => PhpType::array(),
        Hint::Callable(_) => PhpType::callable(),
        Hint::Static(_) => PhpType::static_(),
        Hint::Self_(_) => PhpType::self_(),
        Hint::Parent(_) => PhpType::parent_(),
        Hint::Void(_) => PhpType::void(),
        Hint::Never(_) => PhpType::never(),
        Hint::Float(_) => PhpType::float(),
        Hint::Bool(_) => PhpType::bool(),
        Hint::Integer(_) => PhpType::int(),
        Hint::String(_) => PhpType::string(),
        Hint::Object(_) => PhpType::object(),
        Hint::Mixed(_) => PhpType::mixed(),
        Hint::Iterable(_) => PhpType::iterable(),
        Hint::Parenthesized(paren) => extract_hint_type(paren.hint),
    }
}

/// Recursively flatten nested `Hint::Union` nodes into a flat member list.
fn collect_union_members(hint: &Hint, members: &mut Vec<PhpType>) {
    match hint {
        Hint::Union(union) => {
            collect_union_members(union.left, members);
            collect_union_members(union.right, members);
        }
        other => members.push(extract_hint_type(other)),
    }
}

/// Recursively flatten nested `Hint::Intersection` nodes into a flat member list.
fn collect_intersection_members(hint: &Hint, members: &mut Vec<PhpType>) {
    match hint {
        Hint::Intersection(intersection) => {
            collect_intersection_members(intersection.left, members);
            collect_intersection_members(intersection.right, members);
        }
        other => members.push(extract_hint_type(other)),
    }
}

/// Extract parameter information from a method's parameter list.
///
/// When `content` is provided, default value expressions are extracted
/// from the source text using AST span offsets.  Pass `None` when the
/// source text is not available (the `default_value` field will be `None`
/// for every parameter in that case).
///
/// When `php_version` is `Some`, parameters annotated with
/// `#[PhpStormStubsElementAvailable]` whose version range excludes the
/// target version are filtered out.  When `None`, all parameters are
/// included.
pub(crate) fn extract_parameters(
    parameter_list: &FunctionLikeParameterList,
    content: Option<&str>,
    php_version: Option<PhpVersion>,
    doc_ctx: Option<&DocblockCtx<'_>>,
) -> Vec<ParameterInfo> {
    parameter_list
        .parameters
        .iter()
        .filter(|param| {
            // When a PHP version is configured, skip parameters that are
            // not available for that version.
            if let Some(ver) = php_version
                && let Some(ctx) = doc_ctx
            {
                is_param_available_for_version(param, ctx, ver)
            } else {
                true
            }
        })
        .map(|param| {
            let name = atom(bytes_to_str(param.variable.name));
            let is_variadic = param.ellipsis.is_some();
            let is_reference = param.ampersand.is_some();
            let has_default = param.default_value.is_some();
            let is_required = !has_default && !is_variadic;

            let native_type_hint = param.hint.as_ref().map(|h| extract_hint_type(h));

            // Check for a #[LanguageLevelTypeAware] override on the
            // parameter.  When present, it replaces the native type hint
            // with the version-appropriate type string.
            let mut type_hint = if let Some(ver) = php_version
                && let Some(ctx) = doc_ctx
            {
                extract_language_level_type_for_param(param, ctx, ver)
                    .or_else(|| native_type_hint.clone())
            } else {
                native_type_hint.clone()
            };

            // A parameter with a literal `null` default accepts null
            // regardless of its type hint. This covers the pre-8.4
            // implicit-nullable form `Type $x = null`, which PHP treats
            // as `?Type`. Fold null into the effective type so callers
            // passing null are not flagged. Re-applied after any docblock
            // `@param` merge overwrites `type_hint` (see the callers).
            if param_default_is_null(param) {
                type_hint = type_hint.map(PhpType::or_null);
            }

            let default_value = content.and_then(|src| {
                let dv = param.default_value.as_ref()?;
                let span = dv.value.span();
                let start = span.start.offset as usize;
                let end = span.end.offset as usize;
                let raw = src.get(start..end)?.trim().to_string();
                // Resolve `ClassName::class` to its FQN so that
                // downstream template substitution can find the class.
                if let Some(class_name) = raw.strip_suffix("::class") {
                    Some(resolve_default_class_name(class_name, doc_ctx))
                } else {
                    Some(raw)
                }
            });

            ParameterInfo {
                name,
                is_required,
                native_type_hint: native_type_hint.clone(),
                type_hint,
                description: None,
                default_value,
                is_variadic,
                is_reference,
                closure_this_type: None,
                param_out_type: None,
                callable_invocation: None,
            }
        })
        .collect()
}

/// Whether a parameter's default value is the literal `null`.
///
/// Used to detect the implicit-nullable form `Type $x = null`, which PHP
/// treats as `?Type` (mandatory before 8.4, still permitted after).
pub(crate) fn param_default_is_null(
    param: &function_like::parameter::FunctionLikeParameter<'_>,
) -> bool {
    let Some(dv) = param.default_value.as_ref() else {
        return false;
    };
    match dv.value {
        Expression::Literal(Literal::Null(_)) => true,
        // `Null`, `NULL` and `\null` are the same constant.
        Expression::ConstantAccess(ca) => {
            let name = bytes_to_str(ca.name.value());
            name.strip_prefix('\\')
                .unwrap_or(name)
                .eq_ignore_ascii_case("null")
        }
        _ => false,
    }
}

/// Rewrite a parameter default `Foo::class` (given as `name`, the part
/// before `::class`) to `\Fully\Qualified\Foo::class` using the file's
/// use-map and namespace from [`DocblockCtx`].
///
/// The default is read back as source text at the call site, so the result
/// keeps a leading `\`: an unprefixed `Vendor\Foo` would be qualified with
/// the caller's namespace.  Without imports or a namespace to read it
/// against (the class parse of `update_ast` passes neither) the default is
/// left as written, for the qualification that follows the parse
/// ([`qualify_param_default`]) to settle.
fn resolve_default_class_name(name: &str, doc_ctx: Option<&DocblockCtx<'_>>) -> String {
    match doc_ctx {
        Some(ctx) if !ctx.use_map.is_empty() || ctx.namespace.is_some() => {
            qualify_class_default(name, &ctx.use_map, &ctx.namespace)
        }
        _ => format!("{name}::class"),
    }
}

/// Qualify a `Foo::class` default of `param` written against `use_map` and
/// `namespace`; see [`resolve_default_class_name`].
pub(crate) fn qualify_param_default(
    param: &mut ParameterInfo,
    use_map: &HashMap<String, String>,
    namespace: &Option<String>,
) {
    if let Some(default) = &param.default_value
        && let Some(name) = default.strip_suffix("::class")
        && !name.starts_with('\\')
    {
        param.default_value = Some(qualify_class_default(name, use_map, namespace));
    }
}

/// Rewrite a `self::class` or `parent::class` default of `param` to the
/// class it names in the declaration, `self_fqn` or `parent_fqn`.  The
/// call site reads the default as text, where the keyword would name the
/// caller's class.  A trait passes `None` for both: there the keywords name
/// whichever class uses it.
pub(crate) fn qualify_param_default_keyword(
    param: &mut ParameterInfo,
    self_fqn: Option<&str>,
    parent_fqn: Option<&str>,
) {
    let Some(name) = param
        .default_value
        .as_deref()
        .and_then(|d| d.strip_suffix("::class"))
    else {
        return;
    };
    let target = if name.eq_ignore_ascii_case("self") {
        self_fqn
    } else if name.eq_ignore_ascii_case("parent") {
        parent_fqn
    } else {
        None
    };
    if let Some(fqn) = target {
        param.default_value = Some(format!("\\{}::class", fqn.trim_start_matches('\\')));
    }
}

/// `\Fully\Qualified\Foo::class` for a `Foo::class` default written against
/// `use_map` and `namespace`; see [`resolve_default_class_name`].  `self`,
/// `static` and `parent` are left as written.
fn qualify_class_default(
    name: &str,
    use_map: &HashMap<String, String>,
    namespace: &Option<String>,
) -> String {
    if name.starts_with('\\') || crate::class_lookup::is_class_keyword(name) {
        return format!("{name}::class");
    }
    let fqn = crate::names::resolve_to_fqn(name, use_map, namespace);
    format!("\\{fqn}::class")
}

/// Extract visibility from a set of modifiers.
/// Defaults to `Public` if no visibility modifier is present.
pub(crate) fn extract_visibility<'a>(
    modifiers: impl Iterator<Item = &'a Modifier<'a>>,
) -> Visibility {
    for m in modifiers {
        if m.is_private() {
            return Visibility::Private;
        }
        if m.is_protected() {
            return Visibility::Protected;
        }
        if m.is_public() {
            return Visibility::Public;
        }
    }
    Visibility::Public
}

/// Extract property information from a class member Property node.
pub(crate) fn extract_property_info(
    property: &Property,
    content: Option<&str>,
) -> Vec<PropertyInfo> {
    use mago_syntax::cst::class_like::property::PropertyItem;

    let is_static = property.modifiers().iter().any(|m| m.is_static());
    let is_readonly = property.modifiers().iter().any(|m| m.is_readonly());
    let visibility = extract_visibility(property.modifiers().iter());

    let native_hint = property.hint().map(|h| extract_hint_type(h));

    let mut props = Vec::with_capacity(property.variables().len());
    let mut push_item = |item: &PropertyItem| {
        let default_value = match item {
            PropertyItem::Concrete(c) => content.and_then(|src| {
                let start = c.value.span().start.offset as usize;
                let end = c.value.span().end.offset as usize;
                src.get(start..end).map(Box::from)
            }),
            PropertyItem::Abstract(_) => None,
        };

        let var = item.variable();
        let raw_name = bytes_to_str(var.name).to_string();
        let name = if let Some(stripped) = raw_name.strip_prefix('$') {
            atom(stripped)
        } else {
            atom(&raw_name)
        };

        props.push(PropertyInfo {
            name,
            name_offset: var.span.start.offset,
            type_hint: native_hint.clone(),
            native_type_hint: native_hint.clone(),
            description: None,
            is_static,
            is_readonly,
            visibility,
            deprecation_message: None,
            deprecated_replacement: None,
            see_refs: Vec::new(),
            is_virtual: false,
            source: default_value.map(|value| PropertySource::DeclaredDefault { value }),
        });
    };

    match property {
        Property::Plain(plain) => {
            for item in &plain.items {
                push_item(item);
            }
        }
        Property::Hooked(hooked) => push_item(&hooked.item),
    }

    props
}

use crate::Backend;

impl Backend {
    /// Parse PHP source text and extract class information.
    /// Returns a Vec of ClassInfo for all classes found in the file.
    pub fn parse_php(&self, content: &str) -> Vec<ClassInfo> {
        Self::parse_php_versioned(content, None)
    }

    /// [`parse_php`](Self::parse_php) with the names each class declares
    /// resolved against its `namespace` block, the way an indexed file's
    /// classes are.  For re-parsing patched content whose classes the type
    /// engine reads in place of the file's own.
    pub(crate) fn parse_php_resolved(&self, content: &str) -> Vec<ClassInfo> {
        let _parse_guard = with_parse_cache(content);
        let use_map = self.parse_use_statements(content);
        let constant_imports = self.parse_symbol_imports(content).constants;
        let namespace = self.parse_namespace(content);
        let (mut classes, blocks) = Self::parse_php_classes_by_block(content, None);
        Self::resolve_parent_class_names_by_block(
            &mut classes,
            &blocks,
            &use_map,
            &constant_imports,
            &namespace,
        );
        classes
            .into_iter()
            .map(|(mut cls, ns, _)| {
                cls.file_namespace = ns.as_deref().map(atom);
                cls.cache_fqn();
                cls
            })
            .collect()
    }

    /// Version-aware variant of [`parse_php`].
    ///
    /// When `php_version` is `Some`, elements annotated with
    /// `#[PhpStormStubsElementAvailable]` whose version range excludes
    /// the target version are filtered out during extraction.
    pub fn parse_php_versioned(content: &str, php_version: Option<PhpVersion>) -> Vec<ClassInfo> {
        Self::parse_php_versioned_with_namespaces(content, php_version)
            .into_iter()
            .map(|(cls, _ns)| cls)
            .collect()
    }

    /// Like [`parse_php_versioned`] but returns each class together with
    /// the namespace block it was declared in.
    ///
    /// This is needed by [`parse_and_cache_content_versioned`](crate::resolution)
    /// so that multi-namespace stub files (e.g. PDO.php with both
    /// `namespace { }` and `namespace Pdo { }`) resolve parent class
    /// names against the correct namespace context.
    pub fn parse_php_versioned_with_namespaces(
        content: &str,
        php_version: Option<PhpVersion>,
    ) -> Vec<(ClassInfo, Option<String>)> {
        Self::parse_php_classes_by_block(content, php_version)
            .0
            .into_iter()
            .map(|(cls, ns, _)| (cls, ns))
            .collect()
    }

    /// Like [`parse_php_versioned_with_namespaces`] but also returns the
    /// file's `namespace` blocks, each with its own imports, and tags every
    /// class with the index of the block that declared it (`None` outside
    /// any block).
    ///
    /// The blocks are only returned for a file with more than one; a file
    /// with a single block resolves everything against its file-wide
    /// imports.
    ///
    /// [`parse_php_versioned_with_namespaces`]: Self::parse_php_versioned_with_namespaces
    #[allow(clippy::type_complexity)]
    pub(crate) fn parse_php_classes_by_block(
        content: &str,
        php_version: Option<PhpVersion>,
    ) -> (
        Vec<(ClassInfo, Option<String>, Option<usize>)>,
        Vec<NamespaceSpan>,
    ) {
        with_parsed_program(content, "parse_php", |program, content| {
            let mut use_map = HashMap::new();
            Self::extract_use_statements_from_statements(
                program.statements.iter(),
                &mut use_map,
                None,
            );
            let namespace = Self::extract_namespace_from_statements(program.statements.iter());

            let mut blocks: Vec<NamespaceSpan> = Vec::new();
            for statement in program.statements.iter() {
                if let Statement::Namespace(ns) = statement {
                    let mut block_use_map = HashMap::new();
                    let mut block_symbol_imports = crate::types::SymbolImports::default();
                    Self::extract_use_statements_from_statements(
                        ns.statements().iter(),
                        &mut block_use_map,
                        Some(&mut block_symbol_imports),
                    );
                    let span = ns.span();
                    blocks.push(NamespaceSpan {
                        namespace: ns
                            .name
                            .as_ref()
                            .map(|ident| bytes_to_str(ident.value()).to_string())
                            .filter(|n| !n.is_empty()),
                        start: span.start.offset,
                        end: span.end.offset,
                        use_map: block_use_map,
                        symbol_imports: block_symbol_imports,
                    });
                }
            }
            if blocks.len() <= 1 {
                blocks.clear();
            }

            let doc_ctx = DocblockCtx {
                trivias: program.trivia.as_slice(),
                content,
                php_version,
                use_map,
                namespace,
            };

            let mut result: Vec<(ClassInfo, Option<String>, Option<usize>)> = Vec::new();
            let mut block_index = 0;

            for statement in program.statements.iter() {
                match statement {
                    Statement::Namespace(ns) => {
                        let block_ns: Option<String> = ns
                            .name
                            .as_ref()
                            .map(|ident| bytes_to_str(ident.value()).to_string())
                            .filter(|n| !n.is_empty());

                        let block = blocks.get(block_index).map(|span| (block_index, span));
                        block_index += 1;
                        let block_doc_ctx = block.map(|(_, span)| DocblockCtx {
                            trivias: doc_ctx.trivias,
                            content: doc_ctx.content,
                            php_version: doc_ctx.php_version,
                            use_map: span.use_map.clone(),
                            namespace: span.namespace.clone(),
                        });

                        let mut block_classes = Vec::new();
                        Self::extract_classes_from_statements(
                            ns.statements().iter(),
                            &mut block_classes,
                            Some(block_doc_ctx.as_ref().unwrap_or(&doc_ctx)),
                        );
                        for cls in block_classes {
                            result.push((cls, block_ns.clone(), block.map(|(i, _)| i)));
                        }
                    }
                    statement if Self::is_classlike_extraction_candidate(statement) => {
                        let mut top_classes = Vec::new();
                        Self::extract_classes_from_statements(
                            std::iter::once(statement),
                            &mut top_classes,
                            Some(&doc_ctx),
                        );
                        for cls in top_classes {
                            result.push((cls, None, None));
                        }
                    }
                    _ => {}
                }
            }

            // A class-like declared in two branches of a conditional yields
            // one entry per branch; keep the first so resolution is
            // deterministic (see `dedup_class_likes_first_wins`).
            Self::dedup_class_likes_first_wins(&mut result, |(cls, ns, _)| (cls, ns));

            (result, blocks)
        })
    }

    /// Parse PHP source text and extract standalone function definitions.
    ///
    /// Returns a list of `FunctionInfo` for every `function` declaration
    /// found at the top level (or inside a namespace block).
    pub fn parse_functions(&self, content: &str) -> Vec<FunctionInfo> {
        self.parse_functions_versioned(content, None)
    }

    /// Version-aware variant of [`parse_functions`].
    ///
    /// When `php_version` is `Some`, functions and parameters annotated
    /// with `#[PhpStormStubsElementAvailable]` whose version range
    /// excludes the target version are filtered out.
    pub fn parse_functions_versioned(
        &self,
        content: &str,
        php_version: Option<PhpVersion>,
    ) -> Vec<FunctionInfo> {
        with_parsed_program(content, "parse_functions", |program, content| {
            let mut use_map = HashMap::new();
            Self::extract_use_statements_from_statements(
                program.statements.iter(),
                &mut use_map,
                None,
            );
            let namespace = Self::extract_namespace_from_statements(program.statements.iter());

            let doc_ctx = DocblockCtx {
                trivias: program.trivia.as_slice(),
                content,
                php_version,
                use_map,
                namespace,
            };

            let mut functions = Vec::new();
            Self::extract_functions_from_statements(
                program.statements.iter(),
                &mut functions,
                &None,
                Some(&doc_ctx),
            );
            functions
        })
    }

    /// Parse PHP source text and extract constant names from `define()` calls
    /// and top-level `const` statements.
    ///
    /// Returns a list of `(name, offset, value)` tuples for every
    /// `define('NAME', value)` call or `const NAME = value;` statement
    /// found at the top level, inside namespace blocks, block statements,
    /// or `if` guards.  A `const` is named fully-qualified, a `define()`
    /// exactly as it was written.
    pub fn parse_defines(&self, content: &str) -> Vec<(String, u32, Option<String>)> {
        with_parsed_program(content, "parse_defines", |program, content| {
            let mut defines = Vec::new();
            Self::extract_defines_from_statements(
                program.statements.iter(),
                &mut defines,
                content,
                None,
            );
            defines
        })
    }

    /// Parse PHP source text and extract `use` statement mappings.
    ///
    /// Returns a `HashMap` mapping short (imported) names to their
    /// fully-qualified equivalents.
    ///
    /// For example, `use Klarna\Rest\Resource;` produces
    /// `"Resource" → "Klarna\Rest\Resource"`.
    ///
    /// Handles:
    ///   - Simple use: `use Foo\Bar;`
    ///   - Aliased use: `use Foo\Bar as Baz;`
    ///   - Grouped use: `use Foo\{Bar, Baz};`
    ///   - Mixed grouped use: `use Foo\{Bar, function baz, const QUX};`
    ///     (function / const imports are skipped; see
    ///     [`parse_symbol_imports`](Self::parse_symbol_imports))
    ///   - Use statements inside namespace bodies
    pub(crate) fn parse_use_statements(&self, content: &str) -> HashMap<String, String> {
        with_parsed_program(content, "parse_use_statements", |program, _content| {
            let mut use_map = HashMap::new();
            Self::extract_use_statements_from_statements(
                program.statements.iter(),
                &mut use_map,
                None,
            );
            use_map
        })
    }

    /// Parse PHP source text and extract its `use function` and `use const`
    /// imports.
    pub(crate) fn parse_symbol_imports(&self, content: &str) -> crate::types::SymbolImports {
        with_parsed_program(content, "parse_symbol_imports", |program, _content| {
            let mut imports = crate::types::SymbolImports::default();
            Self::extract_use_statements_from_statements(
                program.statements.iter(),
                &mut HashMap::new(),
                Some(&mut imports),
            );
            imports
        })
    }

    /// Parse PHP source text and extract the declared namespace (if any).
    ///
    /// Returns the namespace string (e.g. `"Klarna\Rest\Checkout"`) or
    /// `None` if the file has no namespace declaration.
    pub(crate) fn parse_namespace(&self, content: &str) -> Option<String> {
        with_parsed_program(content, "parse_namespace", |program, _content| {
            Self::extract_namespace_from_statements(program.statements.iter())
        })
    }
}
