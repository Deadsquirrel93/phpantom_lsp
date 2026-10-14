//! Shared docblock-location helpers for code actions.
//!
//! Several PHPStan quickfixes need to locate the `/** … */` block that sits
//! directly above a function, method, or property signature so they can
//! rewrite or remove tags inside it.  This module owns what they share:
//! [`DocblockAbove`], the block as a byte span with edits that rewrite it
//! or drop lines from it, and [`FunctionDocblock`], the block above a
//! function signature as line indices with edits that set its `@return`.

use std::borrow::Borrow;

use tower_lsp::lsp_types::{Position, Range, TextEdit};

use crate::docblock::strip_docblock_line_delimiters;
use crate::docblock::type_strings::split_type_token;
use crate::text_position::{line_start_byte_offset, offset_to_position};

/// Information about a docblock found above a given line.
pub(crate) struct DocblockAbove {
    /// Byte offset of the start of the docblock (first char of the `/**`
    /// line, including leading whitespace).
    pub(crate) start: usize,
    /// Byte offset just past the end of the docblock (past the `*/` line,
    /// including its trailing newline).
    pub(crate) end: usize,
    /// The raw docblock text including indentation.
    pub(crate) text: String,
}

impl DocblockAbove {
    /// A `TextEdit` that replaces this docblock with `lines`, keeping its
    /// trailing newline.
    pub(crate) fn rewrite_edit<S: Borrow<str>>(&self, content: &str, lines: &[S]) -> TextEdit {
        let mut new_text = lines.join("\n");
        if self.text.ends_with('\n') && !new_text.ends_with('\n') {
            new_text.push('\n');
        }
        self.replace_edit(content, new_text)
    }

    /// A `TextEdit` that rewrites this docblock without the lines at
    /// `lines_to_remove` (indices into its lines).
    ///
    /// A docblock left with nothing but its delimiters and blank `*` lines
    /// is removed entirely, along with its trailing newline.
    pub(crate) fn remove_lines_edit(&self, content: &str, lines_to_remove: &[usize]) -> TextEdit {
        let kept: Vec<&str> = self
            .text
            .lines()
            .enumerate()
            .filter(|(i, _)| !lines_to_remove.contains(i))
            .map(|(_, line)| line)
            .collect();
        let has_content = kept
            .iter()
            .any(|line| !strip_docblock_line_delimiters(line.trim()).is_empty());
        if !has_content && kept.len() <= 3 {
            return self.replace_edit(content, String::new());
        }
        self.rewrite_edit(content, &kept)
    }

    fn replace_edit(&self, content: &str, new_text: String) -> TextEdit {
        TextEdit {
            range: Range {
                start: offset_to_position(content, self.start),
                end: offset_to_position(content, self.end),
            },
            new_text,
        }
    }
}

/// Find the docblock immediately above the given line.
///
/// The diagnostic line is the function/method/property signature.  The
/// docblock (if any) sits directly above it, possibly separated by blank
/// lines or attribute (`#[…]`) lines.
pub(crate) fn find_docblock_above_line(content: &str, line: usize) -> Option<DocblockAbove> {
    let lines: Vec<&str> = content.lines().collect();
    if line == 0 || line > lines.len() {
        return None;
    }

    // Walk backward from the line before the diagnostic to find `*/`.
    let mut doc_end_line = None;
    for i in (0..line).rev() {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.ends_with("*/") {
            doc_end_line = Some(i);
            break;
        }
        // Attributes (#[...]) can appear between docblock and declaration.
        if trimmed.starts_with("#[") {
            continue;
        }
        // Anything else means no docblock.
        break;
    }

    let end_line = doc_end_line?;

    // Walk backward from end_line to find `/**`.
    let mut doc_start_line = None;
    for i in (0..=end_line).rev() {
        let trimmed = lines[i].trim();
        if trimmed.contains("/**") {
            doc_start_line = Some(i);
            break;
        }
        // Should be a `*`-prefixed line or end-of-docblock.
        if !trimmed.starts_with('*') && !trimmed.ends_with("*/") {
            break;
        }
    }

    let start_line = doc_start_line?;

    // Convert line numbers to byte offsets.  Counting the bytes of each
    // line and adding one for its terminator would be a line short per
    // `\r\n`, so the offsets come from the line index instead.
    let start_byte = line_start_byte_offset(content, start_line);
    // The docblock ends where the line after its last one begins, so the
    // trailing newline is part of the span and deleting it takes the
    // whole line with it.
    let end_byte = line_start_byte_offset(content, end_line + 1).min(content.len());

    Some(DocblockAbove {
        start: start_byte,
        end: end_byte,
        text: content.get(start_byte..end_byte).unwrap_or("").to_string(),
    })
}

/// Byte offset of the start of the `/** … */` block sitting above the
/// declaration whose first line starts at `decl_line_start`, if there is one.
///
/// Works entirely in byte offsets, so the result is safe to feed straight
/// into a deletion edit regardless of line endings.  Blank lines between the
/// docblock and the declaration are tolerated and included in the returned
/// span, so deleting `[start, decl_line_end)` removes both.
pub(crate) fn docblock_start_above_offset(content: &str, decl_line_start: usize) -> Option<usize> {
    // Walk backwards over blank lines to the line that closes the docblock.
    let mut line_start = decl_line_start;
    let closing_line = loop {
        let prev = prev_line_start(content, line_start)?;
        let trimmed = content.get(prev..line_start)?.trim();
        if trimmed.ends_with("*/") {
            break prev;
        }
        if !trimmed.is_empty() {
            return None;
        }
        line_start = prev;
    };

    // Walk back over the docblock's own lines to its `/**` opener.
    let mut line_start = closing_line;
    loop {
        let line_end = content[line_start..]
            .find('\n')
            .map_or(content.len(), |i| line_start + i);
        let trimmed = content.get(line_start..line_end)?.trim();
        if trimmed.starts_with("/**") {
            return Some(line_start);
        }
        if !trimmed.starts_with('*') {
            return None;
        }
        line_start = prev_line_start(content, line_start)?;
    }
}

/// Start offset of the line preceding the one starting at `line_start`,
/// or `None` when `line_start` is already the start of the file.
fn prev_line_start(content: &str, line_start: usize) -> Option<usize> {
    if line_start == 0 {
        return None;
    }
    let bytes = content.as_bytes();
    // `line_start - 1` is the newline that terminates the previous line.
    let mut pos = line_start - 1;
    while pos > 0 && bytes[pos - 1] != b'\n' {
        pos -= 1;
    }
    Some(pos)
}

/// Information about the docblock above a function/method, or where
/// to insert one.
pub(crate) struct FunctionDocblock {
    /// Whether an existing docblock was found.
    pub(crate) has_docblock: bool,
    /// The lines of the existing docblock (if any).
    /// Line indices into the source content's line array.
    pub(crate) doc_start_line: usize,
    pub(crate) doc_end_line: usize,
    /// Whether the existing docblock already has a `@return` tag.
    pub(crate) has_return_tag: bool,
    /// Line index of the existing `@return` tag (if any).
    pub(crate) return_tag_line: Option<usize>,
    /// Indentation of the function signature line.
    pub(crate) indent: String,
    /// The signature line the docblock was looked up for, where a new
    /// docblock is inserted when there is none.
    pub(crate) sig_line: usize,
}

impl FunctionDocblock {
    /// The edit that rewrites the type of the existing `@return` tag to
    /// `return_type`, keeping the description after it.
    pub(crate) fn replace_return_type_edit(
        &self,
        lines: &[&str],
        return_type: &str,
    ) -> Option<TextEdit> {
        let tag_line = self.return_tag_line?;
        let line_text = lines[tag_line];
        let at_pos = line_text.find("@return")?;
        let after_return = &line_text[at_pos + "@return".len()..];
        let type_start = after_return
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(after_return.len());
        let (_, description) = split_type_token(&after_return[type_start..]);
        Some(TextEdit {
            range: Range {
                start: Position::new(tag_line as u32, 0),
                end: Position::new(tag_line as u32, line_text.len() as u32),
            },
            new_text: format!(
                "{}@return {}{}",
                &line_text[..at_pos],
                return_type,
                description
            ),
        })
    }

    /// The edit that adds a `@return return_type` tag: into the existing
    /// docblock (expanding a single-line one), or as a new docblock above
    /// the signature when there is none.
    pub(crate) fn add_return_tag_edit(&self, lines: &[&str], return_type: &str) -> TextEdit {
        let indent = &self.indent;
        if !self.has_docblock {
            return insert_at_line_start(
                self.sig_line,
                format!("{indent}/**\n{indent} * @return {return_type}\n{indent} */\n"),
            );
        }

        let doc_end = self.doc_end_line;
        let close_line = lines[doc_end];
        if self.doc_start_line == doc_end {
            // Single-line docblock: convert to multi-line.
            let inner = close_line
                .trim()
                .strip_prefix("/**")
                .and_then(|s| s.strip_suffix("*/"))
                .map(str::trim)
                .unwrap_or("");
            let mut new_doc = format!("{indent}/**\n");
            if !inner.is_empty() {
                new_doc.push_str(&format!("{indent} * {inner}\n{indent} *\n"));
            }
            new_doc.push_str(&format!("{indent} * @return {return_type}\n{indent} */"));
            return TextEdit {
                range: Range {
                    start: Position::new(doc_end as u32, 0),
                    end: Position::new(doc_end as u32, close_line.len() as u32),
                },
                new_text: new_doc,
            };
        }

        // Multi-line docblock: insert before `*/`, separated from a
        // preceding tag other than `@return` or `@throws`.
        let prev_line = if doc_end > self.doc_start_line {
            lines[doc_end - 1].trim()
        } else {
            ""
        };
        let prev_trimmed = prev_line.trim_start_matches('*').trim();
        let needs_separator = prev_trimmed.starts_with('@')
            && !prev_trimmed.starts_with("@return")
            && !prev_trimmed.starts_with("@throws");
        let mut insert_text = String::new();
        if needs_separator {
            insert_text.push_str(&format!("{indent} *\n"));
        }
        insert_text.push_str(&format!("{indent} * @return {return_type}\n"));
        insert_at_line_start(doc_end, insert_text)
    }
}

fn insert_at_line_start(line: usize, new_text: String) -> TextEdit {
    let at = Position::new(line as u32, 0);
    TextEdit {
        range: Range { start: at, end: at },
        new_text,
    }
}

/// Find the docblock above the function/method signature at `sig_line`
/// and extract information about it.
pub(crate) fn find_function_docblock(lines: &[&str], sig_line: usize) -> FunctionDocblock {
    let indent: String = lines
        .get(sig_line)
        .map(|l| l.chars().take_while(|c| c.is_whitespace()).collect())
        .unwrap_or_default();

    if sig_line == 0 {
        return FunctionDocblock {
            has_docblock: false,
            doc_start_line: 0,
            doc_end_line: 0,
            has_return_tag: false,
            return_tag_line: None,
            indent,
            sig_line,
        };
    }

    // Walk backward from the line before the function to find the
    // docblock end.  Skip attribute lines like `#[Override]`, blank
    // lines, and PHP modifier keywords (public, protected, private,
    // static, abstract, final, readonly) that may appear between the
    // docblock and the `function` keyword.
    let mut doc_end_line = None;
    for i in (0..sig_line).rev() {
        let trimmed = lines[i].trim();
        if trimmed.ends_with("*/") {
            doc_end_line = Some(i);
            break;
        }
        if trimmed.starts_with("#[") || trimmed.is_empty() {
            continue;
        }
        if is_php_modifier_line(trimmed) {
            continue;
        }
        // Hit non-docblock, non-attribute content — no docblock.
        break;
    }

    let doc_end_line = match doc_end_line {
        Some(l) => l,
        None => {
            return FunctionDocblock {
                has_docblock: false,
                doc_start_line: 0,
                doc_end_line: 0,
                has_return_tag: false,
                return_tag_line: None,
                indent,
                sig_line,
            };
        }
    };

    let mut doc_start_line = doc_end_line;
    for i in (0..=doc_end_line).rev() {
        let trimmed = lines[i].trim();
        if trimmed.starts_with("/**") {
            doc_start_line = i;
            break;
        }
        if trimmed.starts_with('*') || trimmed.starts_with("/*") {
            continue;
        }
        break;
    }

    // Scan for existing @return tag.
    let mut has_return_tag = false;
    let mut return_tag_line = None;
    for (i, line) in lines
        .iter()
        .enumerate()
        .take(doc_end_line + 1)
        .skip(doc_start_line)
    {
        if line.contains("@return") {
            has_return_tag = true;
            return_tag_line = Some(i);
            break;
        }
    }

    FunctionDocblock {
        has_docblock: true,
        doc_start_line,
        doc_end_line,
        has_return_tag,
        return_tag_line,
        indent,
        sig_line,
    }
}

/// Check whether a trimmed line consists entirely of PHP modifier
/// keywords (public, protected, private, static, abstract, final,
/// readonly).  These can appear on separate lines before `function`.
fn is_php_modifier_line(trimmed: &str) -> bool {
    const MODIFIERS: &[&str] = &[
        "public",
        "protected",
        "private",
        "static",
        "abstract",
        "final",
        "readonly",
    ];
    // Every whitespace-separated token must be a modifier keyword.
    !trimmed.is_empty() && trimmed.split_whitespace().all(|w| MODIFIERS.contains(&w))
}

/// Find the line containing the `function` keyword by walking backward
/// from a given line.
pub(crate) fn find_function_keyword_line(lines: &[&str], from_line: usize) -> Option<usize> {
    (0..=from_line)
        .rev()
        .find(|&i| lines[i].contains("function ") || lines[i].contains("function("))
}
