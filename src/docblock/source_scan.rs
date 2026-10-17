//! Docblock annotations found by scanning raw source text.
//!
//! These scanners read `@var` / `@param` / `@return` annotations straight
//! out of the file content, walking backward from an offset, for callers
//! that need a declared type before (or without) an AST node to hang it
//! on.  The backward walk is confined to the enclosing scope by
//! [`ScopeLines`], so an annotation in a sibling function or class is
//! never picked up.

use super::tags::extract_var_type_with_name;
use super::type_strings::split_type_token;
use crate::php_type::PhpType;

/// Search backward in `content` from `stmt_start` for an inline `/** @var … */`
/// docblock comment and extract the type (and optional variable name).
///
/// Only considers a docblock that is separated from the statement by
/// whitespace alone — no intervening code.
///
/// Returns `(cleaned_type, optional_var_name)` or `None`.
pub fn find_inline_var_docblock(
    content: &str,
    stmt_start: usize,
) -> Option<(PhpType, Option<String>)> {
    let before = content.get(..stmt_start)?;

    // Walk backward past whitespace / newlines.
    let trimmed = before.trim_end();
    if !trimmed.ends_with("*/") {
        return None;
    }

    // Find the matching `/**`.
    let block_end = trimmed.len();
    let open_pos = trimmed.rfind("/**")?;

    // Ensure nothing but whitespace between the start of the line and `/**`.
    let line_start = {
        let s = trimmed.get(..open_pos)?;
        s.rfind('\n').map_or(0, |p| p + 1)
    };
    let prefix = trimmed.get(line_start..open_pos)?;
    if !prefix.chars().all(|c| c.is_ascii_whitespace()) {
        return None;
    }

    let docblock = &trimmed[open_pos..block_end];
    extract_var_type_with_name(docblock)
}

/// Strip the docblock delimiters from a single trimmed source line so the
/// tag text it carries can be matched.
///
/// Each delimiter is optional and removed independently: `/**`, a trailing
/// `*/`, and the leading `*` of a continuation line.  Stripping them
/// independently is what makes a tag written on the same line as the
/// opening `/**` of a *multi-line* docblock readable — chaining
/// `strip_suffix("*/").unwrap_or(trimmed)` onto the prefix strip would
/// restore the `/**` whenever the line does not also close the block.
pub(crate) fn strip_docblock_line_delimiters(trimmed: &str) -> &str {
    let inner = trimmed.strip_prefix("/**").unwrap_or(trimmed);
    let inner = inner.strip_suffix("*/").unwrap_or(inner);
    inner.trim().trim_start_matches('*').trim()
}

/// The text after `@tag` at the start of `inner`, in any of the tag's
/// spellings: plain (`@var`) or vendor-prefixed (`@phpstan-var`,
/// `@psalm-var`).
///
/// The tag has to end there, so `@var` does not match `@variadic`.
fn strip_tag_prefix<'a>(inner: &'a str, tag: &str) -> Option<&'a str> {
    let rest = inner.strip_prefix('@')?;
    let rest = rest
        .strip_prefix("phpstan-")
        .or_else(|| rest.strip_prefix("psalm-"))
        .unwrap_or(rest);
    let rest = rest.strip_prefix(tag)?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then_some(rest)
}

/// The byte offset and length of the first `@var` tag in `line`, in any of
/// its spellings (`@var`, `@phpstan-var`, `@psalm-var`).
pub(crate) fn find_var_tag(line: &str) -> Option<(usize, usize)> {
    ["@phpstan-var", "@psalm-var", "@var"]
        .iter()
        .find_map(|tag| line.find(tag).map(|pos| (pos, tag.len())))
}

/// Pull the inner text out of a `/** ... */` docblock that shares its
/// line with code on either side, e.g.
/// `$x = /** @param T $y */ static function (T $y) {`.
///
/// [`strip_docblock_line_delimiters`] only strips `/**`/`*/` when they
/// sit at the very start/end of the trimmed line, so a docblock preceded
/// or followed by other code on the same line is left untouched and its
/// tags never match. Returns `None` when the line has no `/** ... */`
/// span, or when the span *is* the whole line (that case is already
/// handled by `strip_docblock_line_delimiters`).
fn split_inline_docblock(trimmed: &str) -> Option<&str> {
    let open = trimmed.find("/**")?;
    let after_open = &trimmed[open + 3..];
    let close_rel = after_open.find("*/")?;
    let before = trimmed[..open].trim();
    let after = after_open[close_rel + 2..].trim();
    if before.is_empty() && after.is_empty() {
        return None;
    }
    Some(after_open[..close_rel].trim())
}

/// One source line yielded by [`ScopeLines`].
struct ScopeLine<'a> {
    /// The line with surrounding whitespace trimmed.
    trimmed: &'a str,
    /// Whether the line is a comment or docblock line.  Its braces are not
    /// counted, since a docblock may contain `{` / `}` in array shape type
    /// annotations (e.g. `array{key: string}`).
    is_comment: bool,
    /// Brace depth relative to the starting offset.  Above `0` the line
    /// sits inside a block nested in the scope being searched, so an
    /// annotation on it belongs to that inner scope.
    depth: i32,
}

/// The lines of the scope enclosing an offset, walked backward from it,
/// nearest first.
///
/// Brace depth is tracked so the walk can tell which scope a line is in.
/// Going backward, a `}` enters a block above us (depth increases) and a
/// `{` leaves it (depth decreases).
///
/// The walk ends at the first line that belongs to a sibling scope (a
/// different function, method, or class), since every line above it is
/// foreign too and brace-counting the rest of the file would only produce
/// answers the caller has to discard.
struct ScopeLines<'a> {
    lines: std::iter::Rev<std::str::Lines<'a>>,
    brace_depth: i32,
    min_depth: i32,
    max_depth: i32,
    done: bool,
}

impl<'a> ScopeLines<'a> {
    fn new(search_area: &'a str) -> Self {
        Self {
            lines: search_area.lines().rev(),
            brace_depth: 0,
            min_depth: 0,
            max_depth: 0,
            done: false,
        }
    }
}

impl<'a> Iterator for ScopeLines<'a> {
    type Item = ScopeLine<'a>;

    fn next(&mut self) -> Option<ScopeLine<'a>> {
        if self.done {
            return None;
        }
        let trimmed = self.lines.next()?.trim();

        let is_comment =
            trimmed.starts_with('*') || trimmed.starts_with("/*") || trimmed.starts_with("//");

        let prev_min_depth = self.min_depth;

        if !is_comment {
            let (opens, closes) = count_braces_on_line(trimmed);
            self.brace_depth += closes;
            self.brace_depth -= opens;
        }

        self.min_depth = self.min_depth.min(self.brace_depth);
        self.max_depth = self.max_depth.max(self.brace_depth);

        // Once we have exited our containing scope (min_depth < 0) and
        // re-entered a block close to that level, we are inside a
        // sibling scope (e.g. a different method in the same class).
        //
        // The threshold is `min_depth` rather than `>= 0` because
        // the cursor may be inside a nested block (foreach, if, etc.)
        // that adds extra depth.  When starting inside a foreach in a
        // class method, min_depth reaches -3 (foreach { + method { +
        // class {), so a sibling method body at depth -1 would never
        // reach 0.  Comparing against `min_depth` catches the first rise
        // back toward our exit point.
        if self.min_depth < 0 && self.brace_depth > self.min_depth {
            self.done = true;
            return None;
        }

        // Detect sibling function/method boundaries at the same class
        // nesting level.  When scanning backward we may fully traverse
        // a sibling method body (entering at `}`, exiting at `{`) and
        // return to `brace_depth == 0` — the same level as our own
        // method signature.  The `min_depth < 0` check above only
        // fires when we cross the *enclosing* scope boundary (class
        // `{`), so it misses sibling methods entirely.
        //
        // Fix: once we have entered and fully exited a block at our
        // level (i.e. we saw `brace_depth > 0` and it returned to 0),
        // any `function` keyword at depth 0 is a sibling method
        // signature.  Its docblock belongs to that method, not ours.
        //
        // The `max_depth > 0` guard ensures we only trigger after
        // traversing at least one complete block.  Without it, a
        // scan starting from the parameter list `(` would flag the
        // function's own signature as a sibling boundary, hiding
        // the docblock directly above it.
        //
        // Neither of these two checks fires when a sibling's entire
        // body sits on one line (`function f() { ...; }` or a
        // bodyless `function f();`): its opens and closes cancel out
        // (or are both zero) within a single `count_braces_on_line`
        // call, so `brace_depth` never moves away from `min_depth` and
        // never rises above it either.  Detect that shape separately:
        // a non-comment line sitting at the already-established floor
        // (`brace_depth == min_depth`, unchanged from before this line
        // was processed) is a sibling, since the line that first
        // *carves out* that floor is our own enclosing signature, not
        // a sibling — that one is excluded by requiring `min_depth`
        // itself to be unchanged by this line.
        let at_established_floor = self.min_depth < 0
            && self.brace_depth == self.min_depth
            && self.min_depth == prev_min_depth;

        if !is_comment
            && ((self.brace_depth == 0 && self.max_depth > 0) || at_established_floor)
            && mentions_function_keyword(trimmed)
        {
            // We've hit a sibling function signature.  Any docblock
            // above this point belongs to that function, so stop.
            self.done = true;
            return None;
        }

        Some(ScopeLine {
            trimmed,
            is_comment,
            depth: self.brace_depth,
        })
    }
}

/// Whether `line` declares a named function or method: the `function`
/// keyword, case-insensitively, followed by a name (`&` allowed in
/// between) or by the end of the line.  This covers
/// `public function foo(...)`, `private static function bar(...)`,
/// `function &baz(...)`, and a name wrapped onto the next line, but not a
/// closure (`function (` / `function(`), whose body is part of the scope
/// being searched.
fn mentions_function_keyword(line: &str) -> bool {
    const KEYWORD: &[u8] = b"function";
    let bytes = line.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80;
    bytes.windows(KEYWORD.len()).enumerate().any(|(i, window)| {
        if !window.eq_ignore_ascii_case(KEYWORD) || (i > 0 && is_ident(bytes[i - 1])) {
            return false;
        }
        let rest = &bytes[i + KEYWORD.len()..];
        if rest.first().is_some_and(|&b| is_ident(b)) {
            return false;
        }
        let name_start = rest
            .iter()
            .position(|&b| !b.is_ascii_whitespace() && b != b'&');
        match name_start {
            None => true,
            Some(at) => is_ident(rest[at]),
        }
    })
}

/// Search backward through `content` (up to `before_offset`) for any
/// `/** @var RawType $var_name */` annotation and return the **raw**
/// (uncleaned) type string — including generic parameters like `<User>`.
///
/// This is used by foreach element-type resolution: when iterating over
/// a variable annotated as `list<User>`, we need the raw `list<User>`
/// string so that the generic value type (`User`) can be extracted.
///
/// Only matches annotations that explicitly name the variable
/// (e.g. `/** @var list<User> $users */`) in the enclosing scope; an
/// annotation inside a nested block or a sibling function is not visible.
pub fn find_var_raw_type_in_source(
    content: &str,
    before_offset: usize,
    var_name: &str,
) -> Option<PhpType> {
    let search_area = content.get(..before_offset)?;

    for line in ScopeLines::new(search_area) {
        // Skip annotations that belong to a deeper (inner) scope.
        if line.depth > 0 {
            continue;
        }
        let trimmed = line.trimmed;

        // Quick reject: must mention both the tag (every spelling of
        // which ends in `var`) and the variable.
        if !trimmed.contains("var") || !trimmed.contains(var_name) {
            continue;
        }

        let inner = strip_docblock_line_delimiters(trimmed);

        if let Some(rest) = strip_tag_prefix(inner, "var") {
            let rest = rest.trim_start();
            if rest.is_empty() {
                continue;
            }

            // Extract the full type token (respects `<…>` nesting).
            let (type_token, remainder) = split_type_token(rest);

            // The next token must be our variable name.
            if let Some(name) = remainder.split_whitespace().next()
                && name == var_name
            {
                return Some(PhpType::parse(type_token));
            }
        }
    }

    None
}

/// Search backward through `content` (up to `before_offset`) for any
/// `@var` or `@param` annotation that assigns a raw (uncleaned) type to
/// `$var_name`.
///
/// This combines the logic of [`find_var_raw_type_in_source`] (which looks
/// for `@var Type $var`) and a backward scan for `@param Type $var` in
/// method/function docblocks.
///
/// Returns the first matching raw type string (including generic parameters
/// like `list<User>`), or `None` if no annotation is found.
pub fn find_iterable_raw_type_in_source(
    content: &str,
    before_offset: usize,
    var_name: &str,
) -> Option<PhpType> {
    let search_area = content.get(..before_offset)?;

    // Track the previous non-empty line we saw while scanning backward.
    // This lets us match `/** @var Type */` (no variable name) when the
    // *next* line is an assignment to our variable.
    let mut prev_non_empty_line: Option<&str> = None;

    // A `|Type`/`&Type` union or intersection continuation collected from
    // a line below, waiting to be joined onto the `@param`/`@var` tag
    // that starts the type (see the continuation handling below).
    let mut pending_continuation: Option<String> = None;

    for line in ScopeLines::new(search_area) {
        let trimmed = line.trimmed;
        let is_comment_line = line.is_comment;

        // Skip annotations that belong to a deeper (inner) scope.
        if line.depth > 0 {
            if !trimmed.is_empty() {
                prev_non_empty_line = Some(trimmed);
            }
            continue;
        }

        // ── Union/intersection continued on the next docblock line ──
        // `@param array<int, Widget>` followed by a line starting
        // `|Widget $items` is one type. The continuation carries no
        // `@param`/`@var` keyword of its own, so fold it into a pending
        // tail instead of matching it as a tag; once the scanner reaches
        // the tag line above it, join the two back into one type string.
        if is_comment_line {
            let inner = strip_docblock_line_delimiters(trimmed);
            let is_continuation = inner.starts_with('|') || inner.starts_with('&');

            if is_continuation && pending_continuation.is_some() {
                let existing = pending_continuation.take().unwrap();
                pending_continuation = Some(format!("{inner} {existing}"));
                if !trimmed.is_empty() {
                    prev_non_empty_line = Some(trimmed);
                }
                continue;
            }

            if let Some(tail) = pending_continuation.take() {
                let rest =
                    strip_tag_prefix(inner, "var").or_else(|| strip_tag_prefix(inner, "param"));
                if let Some(rest) = rest {
                    let rest = rest.trim_start();
                    if !rest.is_empty() {
                        let combined = format!("{rest} {tail}");
                        let (type_token, remainder) = split_type_token(&combined);
                        if let Some(name) = remainder.split_whitespace().next()
                            && name == var_name
                        {
                            return Some(PhpType::parse(type_token));
                        }
                    }
                }
                // Not the tag after all — the continuation is discarded
                // and this line falls through to the ordinary checks
                // below.
            }

            if is_continuation && inner.contains(var_name) {
                pending_continuation = Some(inner.to_string());
                if !trimmed.is_empty() {
                    prev_non_empty_line = Some(trimmed);
                }
                continue;
            }
        }

        // ── Named annotation: line mentions the variable name ───────
        // A docblock sharing its line with code (`$x = /** @param T $y
        // */ function ($y) {`) is matched against its own inner text so
        // the annotation is not swallowed by the surrounding code.
        let inline_annotation = split_inline_docblock(trimmed);
        let annotation_source = inline_annotation.unwrap_or(trimmed);
        if annotation_source.contains(var_name) {
            let inner = if inline_annotation.is_some() {
                annotation_source
            } else {
                strip_docblock_line_delimiters(annotation_source)
            };

            // Try @var first, then @param.
            let rest = strip_tag_prefix(inner, "var").or_else(|| strip_tag_prefix(inner, "param"));

            if let Some(rest) = rest {
                let rest = rest.trim_start();
                if !rest.is_empty() {
                    // Extract the full type token (respects `<…>` nesting).
                    let (type_token, remainder) = split_type_token(rest);

                    // The next token must be our variable name.
                    if let Some(name) = remainder.split_whitespace().next()
                        && name == var_name
                    {
                        return Some(PhpType::parse(type_token));
                    }
                }
            }
        }

        // ── No-variable-name annotation: `/** @var Type */` ─────────
        // When the annotation has no variable name, check whether the
        // line immediately following it assigns to our target variable.
        // This handles the common pattern:
        //   /** @var array<int, Customer> */
        //   $thing = [];
        //   $thing[0]->
        if is_comment_line
            && trimmed.contains("var")
            && let Some(next_line) = prev_non_empty_line
            && next_line.contains(var_name)
        {
            // Verify the next line is an assignment to the variable
            // (e.g. `$thing = …;` or `$thing;`).
            let next_trimmed = next_line.trim();
            if next_trimmed.starts_with(var_name)
                && next_trimmed[var_name.len()..].trim_start().starts_with('=')
            {
                let inner = strip_docblock_line_delimiters(trimmed);

                if let Some(rest) = strip_tag_prefix(inner, "var") {
                    let rest = rest.trim_start();
                    if !rest.is_empty() {
                        let (type_token, remainder) = split_type_token(rest);

                        // Only match when there is no variable name in
                        // the annotation (otherwise the named check above
                        // would have matched already).
                        let has_var_name = remainder
                            .split_whitespace()
                            .next()
                            .is_some_and(|t| t.starts_with('$'));
                        if !has_var_name {
                            return Some(PhpType::parse(type_token));
                        }
                    }
                }
            }
        }

        if !trimmed.is_empty() {
            prev_non_empty_line = Some(trimmed);
        }
    }

    None
}

/// Count `{` and `}` characters on a line, skipping those inside string
/// literals.  Returns `(open_count, close_count)`.
fn count_braces_on_line(line: &str) -> (i32, i32) {
    let mut opens = 0i32;
    let mut closes = 0i32;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut prev = '\0';

    for ch in line.chars() {
        if in_single_quote {
            if ch == '\'' && prev != '\\' {
                in_single_quote = false;
            }
            prev = ch;
            continue;
        }
        if in_double_quote {
            if ch == '"' && prev != '\\' {
                in_double_quote = false;
            }
            prev = ch;
            continue;
        }
        match ch {
            '\'' => in_single_quote = true,
            '"' => in_double_quote = true,
            '{' => opens += 1,
            '}' => closes += 1,
            _ => {}
        }
        prev = ch;
    }

    (opens, closes)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── find_var_raw_type_in_source — scope isolation ───────────────

    #[test]
    fn var_docblock_does_not_leak_across_sibling_methods() {
        // A `@var` in one class method must not be visible in another.
        let src = concat!(
            "<?php\n",
            "class A {\n",
            "    public function first(): void {\n",
            "        /** @var object{title: string} $item */\n",
            "        $item = foo();\n",
            "    }\n",
            "}\n",
            "class B {\n",
            "    public function second(): void {\n",
            "        $item->\n", // cursor here
            "    }\n",
            "}\n",
        );
        let cursor = src.find("$item->").unwrap();
        let result = find_var_raw_type_in_source(src, cursor, "$item");
        assert_eq!(
            result, None,
            "@var from A::first() must not leak into B::second()"
        );
    }

    #[test]
    fn var_docblock_does_not_leak_across_sibling_methods_same_class() {
        let src = concat!(
            "<?php\n",
            "class Demo {\n",
            "    public function first(): void {\n",
            "        /** @var Pen $item */\n",
            "        $item = foo();\n",
            "    }\n",
            "    public function second(): void {\n",
            "        $item->\n",
            "    }\n",
            "}\n",
        );
        let cursor = src.find("$item->").unwrap();
        let result = find_var_raw_type_in_source(src, cursor, "$item");
        assert_eq!(
            result, None,
            "@var from first() must not leak into second()"
        );
    }

    #[test]
    fn var_docblock_does_not_leak_when_cursor_inside_nested_block() {
        // The original bug: when the cursor is inside a foreach (or if,
        // while, etc.), the extra nesting depth prevented the sibling
        // scope detection from firing, allowing @var from a method in a
        // completely different class to leak through.
        let src = concat!(
            "<?php\n",
            "class ObjectShapeDemo {\n",
            "    public function demo(): void {\n",
            "        /** @var object{title: string, score: float} $item */\n",
            "        $item = getUnknownValue();\n",
            "    }\n",
            "}\n",
            "class Other {\n",
            "    public function demo(): void {\n",
            "        foreach ($things as $item) {\n",
            "            $item->\n",
            "        }\n",
            "    }\n",
            "}\n",
        );
        let cursor = src.find("$item->").unwrap();
        let result = find_var_raw_type_in_source(src, cursor, "$item");
        assert_eq!(
            result, None,
            "@var from ObjectShapeDemo must not leak into Other when cursor is inside foreach"
        );
    }

    #[test]
    fn var_docblock_found_in_own_scope() {
        let src = concat!(
            "<?php\n",
            "class Demo {\n",
            "    public function demo(): void {\n",
            "        /** @var Pen $item */\n",
            "        $item = foo();\n",
            "        $item->\n",
            "    }\n",
            "}\n",
        );
        let cursor = src.find("$item->").unwrap();
        let result = find_var_raw_type_in_source(src, cursor, "$item");
        assert_eq!(
            result.as_ref().map(|t| t.to_string()),
            Some("Pen".to_string())
        );
    }

    #[test]
    fn var_docblock_found_inside_nested_block_in_own_scope() {
        let src = concat!(
            "<?php\n",
            "class Demo {\n",
            "    public function demo(): void {\n",
            "        /** @var Pen $item */\n",
            "        $item = foo();\n",
            "        if (true) {\n",
            "            $item->\n",
            "        }\n",
            "    }\n",
            "}\n",
        );
        let cursor = src.find("$item->").unwrap();
        let result = find_var_raw_type_in_source(src, cursor, "$item");
        assert_eq!(
            result.as_ref().map(|t| t.to_string()),
            Some("Pen".to_string())
        );
    }

    #[test]
    fn var_docblock_does_not_leak_across_one_line_sibling_function() {
        let src = concat!(
            "<?php\n",
            "/** @var array<Status> $s */\n",
            "function g3(array $s): void { foreach ($s as $x) { doThing($x); } }\n",
            "\n",
            "function g4(Status $s): void { doThing($s); }\n",
        );
        let cursor = src.find("doThing($s)").unwrap();
        let result = find_var_raw_type_in_source(src, cursor, "$s");
        assert_eq!(
            result, None,
            "@var above one-line g3() must not leak into g4()"
        );
    }

    #[test]
    fn var_docblock_seen_past_multi_line_closure() {
        let src = concat!(
            "<?php\n",
            "function demo(): void {\n",
            "    /** @var Pen $item */\n",
            "    $item = make();\n",
            "    $f = function () {\n",
            "        return 1;\n",
            "    };\n",
            "    $item->\n",
            "}\n",
        );
        let cursor = src.find("$item->").unwrap();
        let result = find_var_raw_type_in_source(src, cursor, "$item");
        assert_eq!(
            result.as_ref().map(|t| t.to_string()),
            Some("Pen".to_string()),
            "a closure between the @var and the cursor is not a sibling function"
        );
    }

    // ── find_iterable_raw_type_in_source — scope isolation ──────────

    #[test]
    fn iterable_docblock_does_not_leak_across_sibling_classes_nested() {
        // Same bug scenario for find_iterable_raw_type_in_source:
        // @var in a sibling class method leaks when cursor is nested.
        let src = concat!(
            "<?php\n",
            "class A {\n",
            "    public function demo(): void {\n",
            "        /** @var object{title: string} $item */\n",
            "        $item = foo();\n",
            "    }\n",
            "}\n",
            "class B {\n",
            "    public function demo(): void {\n",
            "        foreach ($things as $x) {\n",
            "            $item->\n",
            "        }\n",
            "    }\n",
            "}\n",
        );
        let cursor = src.find("$item->").unwrap();
        let result = find_iterable_raw_type_in_source(src, cursor, "$item");
        assert_eq!(
            result, None,
            "@var from A::demo() must not leak into B::demo() foreach body"
        );
    }

    #[test]
    fn iterable_param_found_in_own_method_from_nested_block() {
        // @param in the enclosing method's docblock must still be found
        // even when the cursor is inside a nested block.
        let src = concat!(
            "<?php\n",
            "class Demo {\n",
            "    /**\n",
            "     * @param list<Pen> $items\n",
            "     */\n",
            "    public function demo(array $items): void {\n",
            "        foreach ($items as $x) {\n",
            "            // cursor\n",
            "        }\n",
            "    }\n",
            "}\n",
        );
        let cursor = src.find("// cursor").unwrap();
        let result = find_iterable_raw_type_in_source(src, cursor, "$items");
        assert_eq!(
            result.as_ref().map(|t| t.to_string()),
            Some("list<Pen>".to_string())
        );
    }

    #[test]
    fn iterable_param_found_on_docblock_opening_line() {
        // The first tag of a multi-line docblock may share the opening
        // `/**` line.  It must be read just like a continuation line.
        let src = concat!(
            "<?php\n",
            "class Demo {\n",
            "    /** @param list<Pen> $items\n",
            "     *  @return void */\n",
            "    public function demo(array $items): void {\n",
            "        foreach ($items as $x) {\n",
            "            // cursor\n",
            "        }\n",
            "    }\n",
            "}\n",
        );
        let cursor = src.find("// cursor").unwrap();
        let result = find_iterable_raw_type_in_source(src, cursor, "$items");
        assert_eq!(
            result.as_ref().map(|t| t.to_string()),
            Some("list<Pen>".to_string())
        );
    }

    #[test]
    fn iterable_var_found_on_docblock_opening_line() {
        let src = concat!(
            "<?php\n",
            "function demo(): void {\n",
            "    /** @var list<Pen> $items\n",
            "     *  a trailing description */\n",
            "    $items = foo();\n",
            "    foreach ($items as $x) {\n",
            "        // cursor\n",
            "    }\n",
            "}\n",
        );
        let cursor = src.find("// cursor").unwrap();
        assert_eq!(
            find_iterable_raw_type_in_source(src, cursor, "$items")
                .as_ref()
                .map(|t| t.to_string()),
            Some("list<Pen>".to_string())
        );
        assert_eq!(
            find_var_raw_type_in_source(src, cursor, "$items")
                .as_ref()
                .map(|t| t.to_string()),
            Some("list<Pen>".to_string())
        );
    }

    #[test]
    fn docblock_line_delimiters_stripped_independently() {
        assert_eq!(
            strip_docblock_line_delimiters("/** @param string $a */"),
            "@param string $a"
        );
        assert_eq!(
            strip_docblock_line_delimiters("/** @param string $a"),
            "@param string $a"
        );
        assert_eq!(
            strip_docblock_line_delimiters("* @param string $a */"),
            "@param string $a"
        );
        assert_eq!(
            strip_docblock_line_delimiters("*  @param string $a"),
            "@param string $a"
        );
        assert_eq!(strip_docblock_line_delimiters("*/"), "");
    }

    #[test]
    fn iterable_var_found_in_own_scope_nested() {
        let src = concat!(
            "<?php\n",
            "class Demo {\n",
            "    public function demo(): void {\n",
            "        /** @var list<Pen> $items */\n",
            "        $items = foo();\n",
            "        foreach ($items as $x) {\n",
            "            // cursor\n",
            "        }\n",
            "    }\n",
            "}\n",
        );
        let cursor = src.find("// cursor").unwrap();
        let result = find_iterable_raw_type_in_source(src, cursor, "$items");
        assert_eq!(
            result.as_ref().map(|t| t.to_string()),
            Some("list<Pen>".to_string())
        );
    }

    #[test]
    fn iterable_docblock_does_not_leak_across_one_line_sibling_function() {
        // A sibling function whose entire body fits on one line opens and
        // closes its braces within the same `count_braces_on_line` call,
        // so brace_depth never moves.  Its own `@param` must not leak into
        // the next function just because it shares a parameter name.
        let src = concat!(
            "<?php\n",
            "/** @param array<Status> $s */\n",
            "function g3(array $s): void { foreach ($s as $x) { doThing($x); } }\n",
            "\n",
            "function g4(Status $s): void { doThing($s); }\n",
        );
        let cursor = src.find("doThing($s)").unwrap();
        let result = find_iterable_raw_type_in_source(src, cursor, "$s");
        assert_eq!(
            result, None,
            "@param from one-line g3() must not leak into g4()"
        );
    }

    #[test]
    fn iterable_docblock_does_not_leak_across_one_line_sibling_method() {
        let src = concat!(
            "<?php\n",
            "class Demo {\n",
            "    /** @param array<Status> $s */\n",
            "    public function g3(array $s): void { foreach ($s as $x) { doThing($x); } }\n",
            "\n",
            "    public function g4(Status $s): void { doThing($s); }\n",
            "}\n",
        );
        let cursor = src.find("doThing($s)").unwrap();
        let result = find_iterable_raw_type_in_source(src, cursor, "$s");
        assert_eq!(
            result, None,
            "@param from one-line g3() must not leak into g4()"
        );
    }
}
