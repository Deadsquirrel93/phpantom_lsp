/// Core cross-cutting utility functions used throughout the PHPantom
/// server: name resolution against a file's use-map/namespace, panic
/// containment for LSP request handlers, path/URI conversion, and a
/// few other helpers with no single natural module home.
///
/// Related but more cohesive clusters live in dedicated modules:
/// byte-offset/LSP-position conversion in [`crate::text_position`],
/// PHP source text scanning in [`crate::text_scan`], class lookup and
/// subtype checks in [`crate::class_lookup`], external process
/// spawning in [`crate::process`], and `Backend` file/context
/// accessors in [`crate::backend::file_access`].
///
/// Cross-file class/function resolution and name-resolution logic live
/// in the dedicated [`crate::resolution`] module.
///
/// Subject-extraction helpers (walking backwards through characters to
/// find variables, call expressions, balanced parentheses, `new`
/// expressions, etc.) live in [`crate::type_engine::subject_extraction`].
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe, UnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::php_type::PhpType;

/// Resolve an unqualified or partially-qualified PHP class/function name
/// to a fully-qualified name using the file's `use` map and namespace.
///
/// Rules:
///   - Leading `\` — strip it and return (already fully-qualified).
///   - Unqualified (no `\`):
///     1. Check the `use_map` for a direct mapping.
///     2. Prefix with the current namespace.
///     3. Fall back to the bare name (global namespace).
///   - Qualified (contains `\`, no leading `\`):
///     1. Check if the first segment is in the `use_map`; if so, expand it.
///     2. Prefix with the current namespace.
///     3. Fall back to the bare name.
pub(crate) fn resolve_to_fqn(
    name: &str,
    use_map: &HashMap<String, String>,
    namespace: &Option<String>,
) -> String {
    if let Some(stripped) = name.strip_prefix('\\') {
        return stripped.to_string();
    }

    if !name.contains('\\') {
        if let Some(fqn) = use_map.get(name) {
            return fqn.clone();
        }
        if let Some(ns) = namespace {
            return format!("{}\\{}", ns, name);
        }
        return name.to_string();
    }

    let first_segment = name.split('\\').next().unwrap_or(name);
    if let Some(fqn_prefix) = use_map.get(first_segment) {
        let rest = &name[first_segment.len()..];
        return format!("{}{}", fqn_prefix, rest);
    }
    if let Some(ns) = namespace {
        return format!("{}\\{}", ns, name);
    }
    name.to_string()
}

/// Reverse of [`resolve_to_fqn`]: shorten a resolved name to the form the
/// file would reference it by: the `use`-map alias when one imports it,
/// or the bare name when it lives in the file's own namespace.  A
/// qualified name that matches neither gets a leading `\` inside a
/// namespace, which would otherwise read it relative to itself.  A bare
/// name that matches neither comes back as is, since it may be a
/// template parameter rather than a global class; use
/// [`shorten_class_fqn`] for a name known to be a class.
pub(crate) fn shorten_from_fqn(
    fqn: &str,
    use_map: &HashMap<String, String>,
    namespace: &Option<String>,
) -> String {
    let fqn = fqn.trim_start_matches('\\');
    shorten_to_import_or_namespace(fqn, use_map, namespace).unwrap_or_else(|| {
        if namespace.is_some() && fqn.contains('\\') {
            format!("\\{fqn}")
        } else {
            fqn.to_string()
        }
    })
}

/// [`shorten_from_fqn`] for a name known to be a class: one the file can
/// neither import-alias nor reach through its namespace is written fully
/// qualified with a leading `\`, global classes included.
pub(crate) fn shorten_class_fqn(
    fqn: &str,
    use_map: &HashMap<String, String>,
    namespace: &Option<String>,
) -> String {
    let fqn = fqn.trim_start_matches('\\');
    shorten_to_import_or_namespace(fqn, use_map, namespace).unwrap_or_else(|| {
        if namespace.is_some() {
            format!("\\{fqn}")
        } else {
            fqn.to_string()
        }
    })
}

/// Write a resolved type as source text for a file with `use_map` and
/// `namespace`: every name `class_loader` knows as a class is shortened
/// with [`shorten_class_fqn`], and anything else (a template parameter)
/// with [`shorten_from_fqn`].
pub(crate) fn shorten_type_for_file(
    ty: &crate::php_type::PhpType,
    use_map: &HashMap<String, String>,
    namespace: &Option<String>,
    class_loader: &dyn Fn(&str) -> Option<Arc<crate::types::ClassInfo>>,
) -> crate::php_type::PhpType {
    ty.resolve_names(&|name| {
        if crate::class_lookup::load_resolved_name(name, class_loader).is_some() {
            shorten_class_fqn(name, use_map, namespace)
        } else {
            shorten_from_fqn(name, use_map, namespace)
        }
    })
}

fn shorten_to_import_or_namespace(
    fqn: &str,
    use_map: &HashMap<String, String>,
    namespace: &Option<String>,
) -> Option<String> {
    for (alias, imported) in use_map {
        if imported.trim_start_matches('\\') == fqn {
            return Some(alias.clone());
        }
    }
    let rest = fqn
        .strip_prefix(namespace.as_deref()?)
        .and_then(|rest| rest.strip_prefix('\\'))?;
    (!rest.contains('\\')).then(|| rest.to_string())
}

/// Resolve a class name to its FQN via the class loader.
///
/// Returns the FQN from the loaded `ClassInfo` when the loader can find
/// the class, or falls back to the original `name` unchanged.
///
/// **Caveat:** when the loader cannot resolve `name`, the original string
/// is returned as-is.  If `name` is a short (unqualified) class name,
/// the returned value is *not* a FQN — it is the same short name.
/// Callers that need a guaranteed FQN should use [`resolve_to_fqn`]
/// with the file's use-map and namespace instead, falling back to this
/// function only for names that are already expected to be resolvable
/// by the class loader (e.g. names extracted from `::class` expressions
/// or already-resolved type hints).
pub(crate) fn resolve_name_via_loader(
    name: &str,
    class_loader: &dyn Fn(&str) -> Option<Arc<crate::types::ClassInfo>>,
) -> String {
    class_loader(name)
        .map(|cls| cls.fqn().to_string())
        .unwrap_or_else(|| name.to_string())
}

fn find_source_class_name(
    name: &str,
    namespace: Option<&str>,
    local_classes: &[Arc<crate::types::ClassInfo>],
    class_loader: &dyn Fn(&str) -> Option<Arc<crate::types::ClassInfo>>,
) -> Option<String> {
    if !name.contains('\\')
        && let Some(local) = local_classes.iter().find(|c| {
            c.name.eq_ignore_ascii_case(name)
                && match (c.file_namespace.as_deref(), namespace) {
                    (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                    (None, None) => true,
                    _ => false,
                }
        })
    {
        return Some(local.fqn().to_string());
    }
    class_loader(name).map(|cls| cls.fqn().to_string())
}

/// Resolve a source-level (user-typed) class reference for a class named
/// in code (`new Foo`, `Foo::`, `instanceof Foo`, `catch (Foo $e)`) or in a
/// docblock, to the FQN of the class it names, through the source loader.
///
/// The source loader applies PHP's rule (the block's imports, else the
/// current namespace, never a global class), so this only adds a check of
/// the file's own classes: PHP refuses to compile an import that collides
/// with a class declared in the same namespace block, so an unqualified
/// name that matches one always means it.  This is what keeps a file with
/// several `namespace` blocks from resolving a short name in a later block
/// to the same-named class of the first, which is the namespace the
/// file-wide class loader was built for.
///
/// A written class name never falls back to the global scope: inside
/// `namespace App`, an unimported `Foo` is `App\Foo` whether or not that
/// class exists, even when a global `Foo` does.  A name no class answers
/// to is qualified that way too, when nothing in `imports` (the namespace
/// block's `use` table) claims its first segment, so a later lookup of the
/// result cannot land on the global class.
///
/// `imports` is `None` where the caller has no file context to read them
/// from; a miss then comes back as written.
pub(crate) fn resolve_code_class_name(
    name: &str,
    namespace: Option<&str>,
    imports: Option<&HashMap<String, String>>,
    local_classes: &[Arc<crate::types::ClassInfo>],
    class_loader: &dyn Fn(&str) -> Option<Arc<crate::types::ClassInfo>>,
) -> String {
    find_source_class_name(name, namespace, local_classes, class_loader)
        .or_else(|| qualify_unknown_code_name(name, namespace, imports))
        .unwrap_or_else(|| name.to_string())
}

/// The name PHP reads a class name no class answers to as: expanded
/// through the import that claims its first segment, else qualified with
/// the current namespace.  `None` when PHP reads it as written.
fn qualify_unknown_code_name(
    name: &str,
    namespace: Option<&str>,
    imports: Option<&HashMap<String, String>>,
) -> Option<String> {
    expand_import(name, imports).or_else(|| {
        namespace_relative_code_name(name, namespace, imports).map(|ns| format!("{ns}\\{name}"))
    })
}

/// `name` with its first segment replaced by the import that claims it,
/// for a relative name an entry of `imports` claims.
fn expand_import(name: &str, imports: Option<&HashMap<String, String>>) -> Option<String> {
    if name.starts_with('\\') {
        return None;
    }
    let (first_segment, rest) = match name.find('\\') {
        Some(pos) => (&name[..pos], &name[pos..]),
        None => (name, ""),
    };
    lookup_import(imports?, first_segment).map(|target| format!("{target}{rest}"))
}

/// The import `alias` names in `imports`.  Aliases are case-insensitive,
/// like class names, so a `use Lib\Bar;` answers to `bar` too.
pub(crate) fn lookup_import<'m>(
    imports: &'m HashMap<String, String>,
    alias: &str,
) -> Option<&'m String> {
    imports.get(alias).or_else(|| {
        imports
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(alias))
            .map(|(_, target)| target)
    })
}

/// The namespace PHP qualifies a class named in code with, when that is
/// all it can mean: a relative name inside a namespace whose first segment
/// no import claims.  `None` for a fully qualified name, a name in the
/// global namespace, an imported name, or unknown `imports`.
pub(crate) fn namespace_relative_code_name<'a>(
    name: &str,
    namespace: Option<&'a str>,
    imports: Option<&HashMap<String, String>>,
) -> Option<&'a str> {
    let (ns, imports) = (namespace?, imports?);
    if name.starts_with('\\') {
        return None;
    }
    let first_segment = name.split('\\').next().unwrap_or(name);
    lookup_import(imports, first_segment)
        .is_none()
        .then_some(ns)
}

/// Resolve all class names inside a [`PhpType`] to their fully-qualified
/// forms using the class loader.  Scalar/keyword types are left untouched.
///
/// This should be called on any `PhpType` that originates from raw source
/// text (docblock annotations, AST identifiers) before it is stored in a
/// [`ResolvedType`](crate::types::ResolvedType).
pub(crate) fn resolve_php_type_names(
    ty: &crate::php_type::PhpType,
    class_loader: &dyn Fn(&str) -> Option<Arc<crate::types::ClassInfo>>,
) -> crate::php_type::PhpType {
    let resolved = ty.resolve_names(&|name| resolve_name_via_loader(name, class_loader));
    fill_generic_type_defaults(&resolved, class_loader)
}

/// Spell out generic type arguments a declared type left to their
/// `@template` default.
///
/// `@param Test<false> $one` on a class declaring
/// `@template T1 = true  @template T2 = true` binds only `T1`; member
/// lookups already see `T2`'s default because [`build_generic_subs`] fills
/// omitted trailing parameters in, but the annotation's own `PhpType` still
/// carries just the one argument written. Left alone, hover shows
/// `Test<false>` and a comparison against the fully spelled `Test<false,
/// true>` sees two different arities for what should be the same type.
///
/// This walks the type and rebuilds every `Generic` node whose argument
/// count falls short of its class's template parameters, spelling out each
/// omitted parameter the way [`build_generic_subs`] binds it for member
/// substitution.
///
/// Only a parameter that is actually left to something is filled: one with
/// a declared default, or a leading key parameter a short list skips
/// (`Collection<User>` is `Collection<array-key, User>`, `Iterator<Foo>` is
/// `Iterator<mixed, Foo>`). A trailing parameter with neither is left out,
/// as PHPStan leaves it, rather than spelled `mixed`: writing
/// `Iterator<Foo>` out as `Iterator<Foo, mixed>` would move `Foo` into the
/// key slot for every consumer that reads the arguments positionally.
///
/// [`build_generic_subs`]: crate::inheritance::build_generic_subs
fn fill_generic_type_defaults(
    ty: &crate::php_type::PhpType,
    class_loader: &dyn Fn(&str) -> Option<Arc<crate::types::ClassInfo>>,
) -> crate::php_type::PhpType {
    use crate::php_type::{PhpType, TypeKind};

    match ty.raw_kind() {
        TypeKind::Generic(g) => {
            let args: Vec<PhpType> = g
                .args
                .iter()
                .map(|a| fill_generic_type_defaults(a, class_loader))
                .collect();
            // The name is already resolved, so a source-name loader must not
            // resolve a global FQN such as `Iterator` against the file's
            // namespace again.
            let fq_class = crate::class_lookup::load_resolved_name(g.name.as_str(), class_loader);
            let filled_args = fq_class.and_then(|cls| {
                let written = args.len();
                let offset = crate::inheritance::generic_arg_offset(&cls, written);
                let every_omitted_is_left_to_something = cls
                    .template_params
                    .iter()
                    .skip(offset + written)
                    .all(|param| cls.template_param_defaults.contains_key(param));
                (cls.template_params.len() > written && every_omitted_is_left_to_something).then(
                    || {
                        let subs = crate::inheritance::build_generic_subs(&cls, &args);
                        cls.template_params
                            .iter()
                            .map(|param| {
                                subs.get(param.as_str())
                                    .cloned()
                                    .unwrap_or_else(PhpType::mixed)
                            })
                            .collect::<Vec<_>>()
                    },
                )
            });
            PhpType::generic_atom(g.name, filled_args.unwrap_or(args))
        }
        _ => ty.map_children(&|t| fill_generic_type_defaults(t, class_loader)),
    }
}

/// [`resolve_php_type_names`] for a native type hint written in code (a
/// parameter, a closure's return type, a `catch`), where every name is a
/// class: each one follows [`resolve_code_class_name`], so a name no class
/// answers to is still qualified the way PHP reads it.
pub(crate) fn resolve_code_php_type_names(
    ty: &crate::php_type::PhpType,
    namespace: Option<&str>,
    imports: Option<&HashMap<String, String>>,
    local_classes: &[Arc<crate::types::ClassInfo>],
    class_loader: &dyn Fn(&str) -> Option<Arc<crate::types::ClassInfo>>,
) -> crate::php_type::PhpType {
    let resolved = ty.resolve_names(&|name| {
        resolve_code_class_name(name, namespace, imports, local_classes, class_loader)
    });
    fill_generic_type_defaults(&resolved, class_loader)
}

/// [`resolve_php_type_names`] for a type written as source the reader
/// resolves relative to where it stands: a docblock annotation inside a
/// body.
///
/// Every class name follows PHP's rule through the source loader, the same
/// as a `@param` tag (which the parser resolves through the file's use-map
/// and namespace): `@var list<Error>` inside `namespace App` means
/// `App\Error` unless `Error` is imported, never the global `\Error`.
///
/// A name no class answers to is qualified the way
/// [`resolve_code_class_name`] qualifies it, so the global class of the
/// same short name is not read in its place, unless a docblock of
/// `content` (the file the type is written in) declares it as a template
/// parameter or a type alias.  Those keep their spelling for the
/// declaration in scope to read.
pub(crate) fn resolve_source_php_type_names(
    ty: &crate::php_type::PhpType,
    namespace: Option<&str>,
    imports: Option<&HashMap<String, String>>,
    content: &str,
    local_classes: &[Arc<crate::types::ClassInfo>],
    class_loader: &dyn Fn(&str) -> Option<Arc<crate::types::ClassInfo>>,
) -> crate::php_type::PhpType {
    let resolved = ty.resolve_names(&|name| {
        find_source_class_name(name, namespace, local_classes, class_loader)
            .or_else(|| {
                qualify_unknown_code_name(name, namespace, imports)
                    .filter(|_| !declares_docblock_type_name(content, name))
            })
            .unwrap_or_else(|| name.to_string())
    });
    fill_generic_type_defaults(&resolved, class_loader)
}

/// Whether a docblock in `content` declares `name` as a `@template`
/// parameter or a type alias (`@phpstan-type`, `@psalm-type`, or either
/// side of an `@phpstan-import-type … as …`).
///
/// The whole file is searched rather than the declarations around the
/// name: a template kept by a declaration out of its scope is only read
/// the way it was before, while one missed would be qualified into a
/// class that does not exist.  A file in the active parse cache is
/// scanned once per pass (see
/// [`declares_docblock_type_name`](crate::parser::declares_docblock_type_name)).
fn declares_docblock_type_name(content: &str, name: &str) -> bool {
    !name.contains('\\') && crate::parser::declares_docblock_type_name(content, name)
}

/// [`declares_docblock_type_name`] for text outside the parse cache: a
/// search for `name` alone, cheaper for one lookup than collecting every
/// declared name.
pub(crate) fn scan_declares_docblock_type_name(content: &str, name: &str) -> bool {
    let bytes = content.as_bytes();
    memchr::memmem::find_iter(bytes, name.as_bytes()).any(|pos| {
        // A declared name follows the tag (or `as`) after whitespace.
        if pos == 0 || !bytes[pos - 1].is_ascii_whitespace() {
            return false;
        }
        let line_start = content[..pos].rfind('\n').map_or(0, |i| i + 1);
        let line_end = content[pos..].find('\n').map_or(content.len(), |i| pos + i);
        let mut found = false;
        line_declared_type_names(&content[line_start..line_end], |declared| {
            found |= declared == name;
        });
        found
    })
}

/// Every name a docblock in `content` declares as a template parameter or
/// a type alias; see [`declares_docblock_type_name`].
pub(crate) fn docblock_declared_type_names(content: &str) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    for line in content.lines().filter(|line| line.contains('@')) {
        line_declared_type_names(line, |name| {
            names.insert(name.to_string());
        });
    }
    names
}

fn line_declared_type_names<'a>(line: &'a str, mut found: impl FnMut(&'a str)) {
    let identifier = |word: &'a str| {
        let end = word
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || !c.is_ascii()))
            .unwrap_or(word.len());
        &word[..end]
    };
    for tag in line.split('@').skip(1) {
        let (tag, rest) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
        let mut words = rest.split_whitespace();
        if tag.ends_with("import-type") {
            // `@phpstan-import-type Alias from Source as Local`
            if let Some(alias) = words.next() {
                found(identifier(alias));
            }
            if let Some(local) = words.skip_while(|w| *w != "as").nth(1) {
                found(identifier(local));
            }
        } else if (tag.ends_with("template")
            || tag.ends_with("template-covariant")
            || tag.ends_with("template-contravariant")
            || tag == "type"
            || tag.ends_with("-type"))
            && let Some(name) = words.next()
        {
            found(identifier(name));
        }
    }
}

/// Run `f` inside [`panic::catch_unwind`], logging and swallowing any
/// panic.
///
/// Returns `Some(value)` on success and `None` on panic.  The error
/// message includes `label` (the operation name, e.g. `"hover"` or
/// `"goto_definition"`), `uri`, and the optional cursor `position`.
///
/// This centralises the boilerplate that every LSP handler uses to
/// guard against stack overflows and unexpected panics in the
/// resolution pipeline.
///
/// # Examples
///
/// ```ignore
/// let result = catch_panic("hover", uri, Some(position), || {
///     self.handle_hover(uri, content, position)
/// });
/// ```
pub(crate) fn catch_panic<T>(
    label: &str,
    uri: &str,
    position: Option<Position>,
    f: impl FnOnce() -> T + UnwindSafe,
) -> Option<T> {
    match panic::catch_unwind(f) {
        Ok(value) => Some(value),
        Err(_) => {
            if let Some(pos) = position {
                tracing::error!(
                    "PHPantom: panic during {} at {}:{}:{}",
                    label,
                    uri,
                    pos.line,
                    pos.character
                );
            } else {
                tracing::error!("PHPantom: panic during {} at {}", label, uri);
            }
            None
        }
    }
}

/// Convenience wrapper around [`catch_panic`] for closures that
/// capture `&self` or other non-[`UnwindSafe`] references.
///
/// Wraps `f` in [`AssertUnwindSafe`] before forwarding to
/// [`catch_panic`].  This is safe in our context because a panic
/// during LSP handling never leaves shared state in an inconsistent
/// state (the worst case is a stale cache entry).
pub(crate) fn catch_panic_unwind_safe<T>(
    label: &str,
    uri: &str,
    position: Option<Position>,
    f: impl FnOnce() -> T,
) -> Option<T> {
    catch_panic(label, uri, position, AssertUnwindSafe(f))
}

/// Convert a filesystem path to a properly percent-encoded `file://` URI string.
///
/// This **must** be used instead of `format!("file://{}", path.display())`
/// everywhere in the codebase.  The `format!` approach produces raw,
/// un-encoded paths (e.g. `file:///home/user/My Project/Foo.php`) while
/// LSP clients send URIs through the `Url` type which percent-encodes
/// special characters (e.g. `file:///home/user/My%20Project/Foo.php`).
/// When both forms end up as keys in `symbol_maps`, the same file is
/// indexed twice and every Find References result is duplicated.
///
/// Falls back to the raw `format!` form only when `Url::from_file_path`
/// fails (non-absolute paths on some platforms), which should never
/// happen in practice.
pub(crate) fn path_to_uri(path: &Path) -> String {
    // Canonicalize relative paths to absolute so that
    // `Url::from_file_path` never fails due to a missing leading `/`.
    let abs_path;
    let effective = if path.is_relative() {
        abs_path = std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf());
        abs_path.as_path()
    } else {
        path
    };
    Url::from_file_path(effective)
        .map(|u| u.to_string())
        .unwrap_or_else(|()| format!("file://{}", effective.display()))
}

/// The URI of the file at `internal_path` inside the phar at `phar_path`,
/// e.g. `phar:///path/to/phpstan.phar/src/Type/Type.php`.
pub(crate) fn phar_entry_uri(phar_path: &Path, internal_path: &str) -> String {
    format!("phar://{}/{}", phar_path.display(), internal_path)
}

/// Recursively collect all `.php` files under a directory, respecting
/// `.gitignore` rules and skipping hidden directories (`.git`,
/// `.idea`, etc.).
///
/// Uses the `ignore` crate's `WalkBuilder` for gitignore-aware
/// traversal.  This is consistent with the other workspace walkers
/// (`scan_workspace_fallback_full`, `crate::classmap_scanner::collect_php_files_gitignore`).
///
/// Used by Go-to-implementation (Phase 5) which walks PSR-4 source
/// directories.
///
/// `vendor_dir_paths` contains absolute paths of all known vendor
/// directories (one per subproject in monorepo mode).  Any directory
/// whose absolute path matches one of these is skipped regardless of
/// `.gitignore` content.
///
/// Silently skips directories and files that cannot be read (e.g.
/// permission errors, broken symlinks).
pub(crate) fn collect_php_files(
    dir: &Path,
    vendor_dir_paths: &[PathBuf],
    filters: &std::sync::Arc<crate::classmap_scanner::IndexFilters>,
) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let walker = crate::classmap_scanner::workspace_walk_builder(
        dir,
        std::sync::Arc::new(vendor_dir_paths.to_vec()),
        std::sync::Arc::clone(filters),
        false,
        crate::classmap_scanner::LinkClaims::new([dir.to_path_buf()], None),
    )
    .build();

    for entry in walker.flatten() {
        let path = entry.path();
        if path.is_file() && filters.is_php_file(path) {
            result.push(path.to_path_buf());
        }
    }

    result
}

/// Drop the entries `keep` marks `false`, in place.
///
/// `keep` is indexed the same as `items`, which is what a pairwise pass
/// over the whole list produces. `Vec::retain` cannot see the index it is
/// at, so a counter walks alongside it.
pub(crate) fn retain_by_mask<T>(items: &mut Vec<T>, keep: &[bool]) {
    debug_assert_eq!(items.len(), keep.len());
    let mut index = 0;
    items.retain(|_| {
        let retain = keep.get(index).copied().unwrap_or(true);
        index += 1;
        retain
    });
}

/// Extract the short (unqualified) class name from a potentially
/// fully-qualified name.
///
/// For example, `"Illuminate\\Support\\Collection"` → `"Collection"`,
/// and `"Collection"` → `"Collection"`.
pub(crate) fn short_name(name: &str) -> &str {
    name.rsplit('\\').next().unwrap_or(name)
}

/// Strip the leading fully-qualified-name backslash from a PHP name.
///
/// `"\\Foo\\Bar"` -> `"Foo\\Bar"`, `"Foo"` -> `"Foo"`.
pub(crate) fn strip_fqn_prefix(name: &str) -> &str {
    name.strip_prefix('\\').unwrap_or(name)
}

/// Strip the surrounding quotes from a PHP string literal and unescape its
/// body, returning the runtime string value.
///
/// This matters when the literal names a class: `'Foo\\Bar'` is the class
/// `Foo\Bar` at runtime, so the doubled backslash in the source text must be
/// collapsed before the value is used as a type/class name. Backslash escapes
/// are resolved in a single left-to-right pass (sequential `replace` calls are
/// order-dependent and mis-handle adjacent escapes). In a single-quoted string
/// only `\\` and `\'` are special; in a double-quoted string only `\\` and
/// `\"` are recognised here (other backslash sequences are kept literal, which
/// is sufficient for class names). Returns `None` when `raw` is not a quoted
/// string literal.
pub(crate) fn unescape_php_string_literal(raw: &str) -> Option<String> {
    let (body, double_quoted) =
        if let Some(b) = raw.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
            (b, false)
        } else {
            let b = raw.strip_prefix('"').and_then(|r| r.strip_suffix('"'))?;
            (b, true)
        };

    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('\'') if !double_quoted => out.push('\''),
                Some('"') if double_quoted => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Build a fully-qualified name from a short name and an optional namespace.
///
/// `("Foo", Some("App\\Models"))` → `"App\\Models\\Foo"`,
/// `("Foo", None)` → `"Foo"`.
pub(crate) fn build_fqn(short_name: &str, namespace: Option<&str>) -> String {
    match namespace {
        Some(ns) if !ns.is_empty() => format!("{}\\{}", ns, short_name),
        _ => short_name.to_string(),
    }
}

/// Strip trailing PHP visibility/modifier keywords from a string.
///
/// Given a string like `"  /** ... */\n    public static"`, returns
/// `"  /** ... */"` (after stripping `static` and `public`).
///
/// Recognised modifiers: `public`, `protected`, `private`, `static`,
/// `abstract`, `final`, `readonly`.
pub(crate) fn strip_trailing_modifiers(s: &str) -> &str {
    const MODIFIERS: &[&str] = &[
        "public",
        "protected",
        "private",
        "static",
        "abstract",
        "final",
        "readonly",
    ];

    let mut result = s;
    loop {
        let trimmed = result.trim_end();
        let mut found = false;
        for &kw in MODIFIERS {
            if let Some(prefix) = trimmed.strip_suffix(kw) {
                // Make sure the keyword isn't part of a larger identifier.
                if prefix.is_empty()
                    || prefix
                        .as_bytes()
                        .last()
                        .is_some_and(|&b| !b.is_ascii_alphanumeric() && b != b'_')
                {
                    result = prefix;
                    found = true;
                    break;
                }
            }
        }
        if !found {
            break;
        }
    }
    result.trim_end()
}

/// Check if a line contains the `function` keyword as a standalone word
/// (not part of a larger identifier like `$functionality`).
pub(crate) fn contains_function_keyword(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(pos) = trimmed.find("function") else {
        return false;
    };
    let before_ok = pos == 0 || trimmed.as_bytes()[pos - 1].is_ascii_whitespace();
    let after_pos = pos + "function".len();
    let after_ok = after_pos >= trimmed.len()
        || !trimmed.as_bytes()[after_pos].is_ascii_alphanumeric()
            && trimmed.as_bytes()[after_pos] != b'_';
    before_ok && after_ok
}

/// Infer a [`PhpType`] from a literal expression string.
///
/// Recognises integer, float, boolean, null, and string literals as
/// well as empty arrays (`[]`).  Returns `None` for anything that
/// is not a simple literal — callers should fall back to the full
/// type resolver for those cases.
///
/// This is the shared core used by:
/// - `code_actions::phpstan::fix_return_type::infer_type_from_literal`
///   (extended wrapper that also handles `new` expressions and array
///   literal contents)
/// - `code_actions::extract_constant::literal_type_name`
/// - `parser::classes` (Team 3, future)
pub(crate) fn infer_type_from_literal(expr: &str) -> Option<PhpType> {
    // Integer literal (decimal, hex, octal, binary — all parse as i64
    // after stripping underscores for PHP 7.4+ numeric separators).
    let clean = expr.replace('_', "");
    if clean.parse::<i64>().is_ok() {
        return Some(PhpType::int());
    }
    // Hex / octal / binary that i64 doesn't cover directly.
    if (clean.starts_with("0x") || clean.starts_with("0X"))
        && i64::from_str_radix(&clean[2..], 16).is_ok()
    {
        return Some(PhpType::int());
    }
    if (clean.starts_with("0b") || clean.starts_with("0B"))
        && i64::from_str_radix(&clean[2..], 2).is_ok()
    {
        return Some(PhpType::int());
    }
    // Octal
    if clean.starts_with('0')
        && clean.len() > 1
        && clean[1..].chars().all(|c| c.is_ascii_digit())
        && i64::from_str_radix(&clean[1..], 8).is_ok()
    {
        return Some(PhpType::int());
    }

    // Float literal (must contain `.`, `e`, or `E` to distinguish from int).
    if (clean.contains('.') || clean.contains('e') || clean.contains('E'))
        && clean.parse::<f64>().is_ok()
    {
        return Some(PhpType::float());
    }

    // Negative numeric literals.
    if let Some(stripped) = expr.strip_prefix('-') {
        let abs = stripped.trim_start();
        if let Some(inner) = infer_type_from_literal(abs)
            && (inner.is_int() || inner.is_float())
        {
            return Some(inner);
        }
    }

    // Boolean literals.
    if expr.eq_ignore_ascii_case("true") || expr.eq_ignore_ascii_case("false") {
        return Some(PhpType::bool());
    }

    // Null.
    if expr.eq_ignore_ascii_case("null") {
        return Some(PhpType::null());
    }

    // String literals (single- or double-quoted).
    if (expr.starts_with('\'') && expr.ends_with('\''))
        || (expr.starts_with('"') && expr.ends_with('"'))
    {
        return Some(PhpType::string());
    }

    // Empty array literal.
    if expr == "[]" {
        return Some(PhpType::array());
    }

    // Not a simple literal.
    None
}

/// Find the concrete method body block that contains `offset` within
/// the given class-like members.  Returns `None` if no method body
/// spans the offset.
///
/// This is the shared kernel behind "find enclosing body" operations
/// used by extract-function, property-assignment narrowing, and
/// similar features that need to locate the method body surrounding
/// the cursor.
pub(crate) fn find_enclosing_method_block_in_members<'a>(
    members: impl Iterator<Item = &'a mago_syntax::cst::class_like::member::ClassLikeMember<'a>>,
    offset: u32,
) -> Option<&'a mago_syntax::cst::block::Block<'a>> {
    use mago_syntax::cst::class_like::member::ClassLikeMember;
    use mago_syntax::cst::class_like::method::MethodBody;

    for member in members {
        if let ClassLikeMember::Method(method) = member
            && let MethodBody::Concrete(block) = &method.body
        {
            let body_start = block.left_brace.start.offset;
            let body_end = block.right_brace.end.offset;
            if offset >= body_start && offset <= body_end {
                return Some(block);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_string_literal_collapses_namespace_separators() {
        // Single-quoted `'Foo\\Bar'` names the class `Foo\Bar` at runtime.
        assert_eq!(
            unescape_php_string_literal(r"'Foo\\Bar'").as_deref(),
            Some(r"Foo\Bar")
        );
        // Double-quoted spelling collapses the same way.
        assert_eq!(
            unescape_php_string_literal(r#""App\\Models\\User""#).as_deref(),
            Some(r"App\Models\User")
        );
    }

    #[test]
    fn unescape_string_literal_handles_quote_and_lone_backslash() {
        // `\'` produces a literal quote inside a single-quoted string.
        assert_eq!(
            unescape_php_string_literal(r"'a\'b'").as_deref(),
            Some("a'b")
        );
        // A lone backslash before a non-special char stays literal.
        assert_eq!(
            unescape_php_string_literal(r"'a\nb'").as_deref(),
            Some(r"a\nb")
        );
        // An empty string literal unescapes to an empty string.
        assert_eq!(unescape_php_string_literal("''").as_deref(), Some(""));
    }

    #[test]
    fn unescape_string_literal_rejects_unquoted_input() {
        assert_eq!(unescape_php_string_literal("bare"), None);
        assert_eq!(unescape_php_string_literal("'unterminated"), None);
    }

    /// `ty` written inside `namespace App` of `content`, with `imports` and
    /// no class answering to any name.
    fn resolve_unknown_docblock_type(ty: &str, imports: &[(&str, &str)], content: &str) -> String {
        let imports: HashMap<String, String> = imports
            .iter()
            .map(|(alias, fqn)| (alias.to_string(), fqn.to_string()))
            .collect();
        resolve_source_php_type_names(
            &crate::php_type::PhpType::parse(ty),
            Some("App"),
            Some(&imports),
            content,
            &[],
            &|_| None,
        )
        .to_string()
    }

    #[test]
    fn unknown_docblock_name_is_qualified_like_code() {
        assert_eq!(
            resolve_unknown_docblock_type("Bar", &[], "<?php"),
            r"App\Bar"
        );
        assert_eq!(
            resolve_unknown_docblock_type("list<Bar>", &[("Bar", r"Lib\Bar")], "<?php"),
            r"list<Lib\Bar>"
        );
        assert_eq!(
            resolve_unknown_docblock_type(r"\Bar", &[], "<?php"),
            r"\Bar"
        );
    }

    #[test]
    fn docblock_templates_and_aliases_keep_their_spelling() {
        for content in [
            "/**\n * @template T\n */",
            "/** @template-covariant T of object */",
            "/** @psalm-template T as object */",
            "/** @phpstan-type T array{a: int} */",
            "/** @psalm-type T=array{a: int} */",
            "/** @phpstan-import-type T from Other */",
            "/** @phpstan-import-type Shape from Other as T */",
        ] {
            assert_eq!(
                resolve_unknown_docblock_type("T", &[], content),
                "T",
                "{content}"
            );
        }
        for content in [
            "/** @template-extends T<int> */",
            "/** @var T $t */",
            "/** @template TT */",
        ] {
            assert_eq!(
                resolve_unknown_docblock_type("T", &[], content),
                r"App\T",
                "{content}"
            );
        }
    }

    #[test]
    fn collect_php_files_follows_interior_symlink() {
        // Go-to-implementation's walker keeps the same symlink contract
        // as the other workspace walkers (issue #383).
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        let real = dir.path().join("real");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("Hidden.php"), "<?php\n").unwrap();

        let link = root.join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&real, &link).unwrap();

        let files = collect_php_files(&root, &[], &crate::classmap_scanner::IndexFilters::empty());
        let linked = files
            .iter()
            .find(|p| p.ends_with("Hidden.php"))
            .unwrap_or_else(|| panic!("linked file must be indexed: {files:?}"));
        assert!(
            linked.starts_with(&link),
            "paths must keep the symlink spelling: {linked:?} vs {link:?}"
        );
    }

    #[test]
    fn collect_php_files_walks_a_link_target_once() {
        // Two links to one tree must not make go-to-implementation offer
        // the same class twice under two spellings.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("ws");
        let real = dir.path().join("real");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("Hidden.php"), "<?php\n").unwrap();

        for name in ["a", "b"] {
            let link = root.join(name);
            #[cfg(unix)]
            std::os::unix::fs::symlink(&real, &link).unwrap();
            #[cfg(windows)]
            std::os::windows::fs::symlink_dir(&real, &link).unwrap();
        }

        let files = collect_php_files(&root, &[], &crate::classmap_scanner::IndexFilters::empty());
        assert_eq!(
            files.len(),
            1,
            "the linked tree must be reported once, not once per link: {files:?}"
        );
    }
}
