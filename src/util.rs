//! Core cross-cutting utility functions used throughout the PHPantom
//! server: panic containment for LSP request handlers, path/URI
//! conversion, and a few other helpers with no single natural module
//! home.
//!
//! Related but more cohesive clusters live in dedicated modules:
//! reading a class name against a file's imports and namespace in
//! [`crate::names`], byte-offset/LSP-position conversion in
//! [`crate::text_position`], PHP source text scanning in
//! [`crate::text_scan`], class lookup and subtype checks in
//! [`crate::class_lookup`], external process spawning in
//! [`crate::process`], and `Backend` file/context accessors in
//! [`crate::backend::file_access`].
//!
//! Cross-file class/function resolution lives in the dedicated
//! [`crate::resolution`] module.
//!
//! Subject-extraction helpers (walking backwards through characters to
//! find variables, call expressions, balanced parentheses, `new`
//! expressions, etc.) live in [`crate::type_engine::subject_extraction`].
use std::panic::{self, AssertUnwindSafe, UnwindSafe};
use std::path::{Path, PathBuf};

use tower_lsp::lsp_types::*;

use crate::php_type::PhpType;

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
