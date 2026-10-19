//! Closing the brackets an unfinished expression leaves open before a `}`
//! or at the end of the file.
//!
//! When mago's parser expects an expression and finds a `}`, as in a call
//! whose `)` has not been typed yet (`new Widget(` on the line above the
//! method's `}`), it takes the `}` for the expression.  The braces stop
//! balancing from there: the method body closes on the class's `}`, the
//! class runs to the end of the file, and the parser drops it whole, with
//! every name it uses.
//!
//! The parser reports each bracket it found open at the token after the
//! `}` it took, innermost first.  [`close_open_brackets`] parses the file again
//! with those closers written over the whitespace after the last code
//! before the `}` it took, which leaves every offset where it was, so the
//! reparse describes the text the caller holds.
//!
//! A top-level statement the end of the file cuts off, as the newest
//! route in a routes file is while it is typed, is dropped too, even once
//! its brackets are closed if its `;` is missing.  The parser reports what
//! it found open at the end of the file, and the reparse writes the
//! closers and a `;` over the whitespace the file ends in.  A file usually
//! ends in a single newline, which has room for the `;` alone, and the
//! parser takes a `;` for the closers of a call or array that already
//! holds something.

use mago_allocator::{Arena, LocalArena};
use mago_syntax::cst::{Program, Trivia};
use mago_syntax::error::{Expected, ParseError};
use mago_syntax::parser::parse_file_content;
use mago_syntax::token::TokenKind;

/// Parse `content` again with the brackets its parse, `program`, found
/// open before a `}` it took for an expression or at the end of the file
/// closed, or `None` when it found none it could close.
///
/// Returns the reparse with the text it read, both allocated in `arena`.
/// Its `errors` describe that text, so a caller reporting syntax errors
/// takes them from `program`.
pub(super) fn close_open_brackets<'a>(
    arena: &'a LocalArena,
    program: &'a Program<'a>,
    content: &str,
) -> Option<(&'a Program<'a>, &'a str)> {
    let mut text = content.to_string();
    let mut parse = program;
    let mut closed_up_to = 0;
    let mut kept = None;
    loop {
        // Closing a bracket can let the parser read on to one it found open
        // further down.  Each round closes brackets further down, so this
        // ends once none is left.
        let patches: Vec<_> = open_brackets(parse, &text)
            .filter(|patch| patch.at >= closed_up_to)
            .collect();
        if patches.is_empty() {
            return kept;
        }
        for patch in &patches {
            text.replace_range(patch.at..patch.at + patch.closers.len(), &patch.closers);
            closed_up_to = closed_up_to.max(patch.at + patch.closers.len());
        }
        let source: &str = arena.alloc_str(&text);
        // The text is still malformed, and the parser panics on some
        // malformed text.  A parse that got through stands then.
        let Ok(reparsed) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            parse_file_content(arena, program.file_id, source.as_bytes())
        })) else {
            return kept;
        };
        kept = Some((reparsed, source));
        parse = reparsed;
    }
}

/// The closers to write at `at`, over as many bytes of whitespace.
struct Patch {
    at: usize,
    closers: String,
}

/// The brackets the parse of `content` found open before each `}` it took
/// for an expression, and at the end of the file, with whitespace to write
/// their closers over.
fn open_brackets<'p>(
    program: &'p Program<'p>,
    content: &'p str,
) -> impl Iterator<Item = Patch> + 'p {
    let errors = program.errors;
    let trivia = program.trivia.as_slice();
    let bytes = content.as_bytes();
    let before_brace = errors.iter().enumerate().filter_map(move |(i, error)| {
        let brace = brace_taken(error)?;
        let closers = closers_reported(&errors[i + 1..]);
        if closers.is_empty() {
            return None;
        }
        let at = code_end(trivia, bytes, brace);
        let room = bytes[at..brace]
            .iter()
            .take_while(|b| b.is_ascii_whitespace())
            .count();
        (room >= closers.len()).then_some(Patch { at, closers })
    });
    before_brace.chain(at_end_of_file(errors, trivia, bytes))
}

/// The offset of the `}` the parser took for an expression, when `error`
/// reports one.
fn brace_taken(error: &ParseError) -> Option<usize> {
    match error {
        ParseError::UnexpectedToken(Expected::OneOf([]), TokenKind::RightBrace, span) => {
            Some(span.start.offset as usize)
        }
        _ => None,
    }
}

/// The closers and `;` ending a statement the end of the file cuts off,
/// when the parser reports nothing else open there.
fn at_end_of_file(errors: &[ParseError], trivia: &[Trivia<'_>], bytes: &[u8]) -> Option<Patch> {
    let first = errors
        .iter()
        .position(|error| matches!(error, ParseError::UnexpectedEndOfFile(..)))?;
    // Brackets left open before a `}` the parser took are reported at the
    // end of the file too, and closed before that `}`.
    if first > 0 && brace_taken(&errors[first - 1]).is_some() {
        return None;
    }
    let reported = &errors[first..];
    let mut closers = closers_reported(reported);
    let ends_here = if closers.is_empty() {
        matches!(
            reported,
            [ParseError::UnexpectedEndOfFile(Expected::OneOf(kinds), _, _)]
                if kinds.contains(&TokenKind::Semicolon)
        )
    } else {
        closers.len() == reported.len()
    };
    if !ends_here {
        return None;
    }
    let at = code_end(trivia, bytes, bytes.len());
    let room = bytes[at..]
        .iter()
        .take_while(|b| b.is_ascii_whitespace())
        .count();
    if room == 0 {
        return None;
    }
    if room > closers.len() {
        closers.push(';');
    } else {
        closers = ";".to_string();
    }
    Some(Patch { at, closers })
}

/// The closers of the brackets the leading run of `errors` reports open at
/// one token, innermost first.
fn closers_reported(errors: &[ParseError]) -> String {
    let mut closers = String::new();
    let mut place = None;
    for error in errors {
        let (kind, at) = match error {
            ParseError::UnexpectedToken(Expected::Exactly(kind), _, span) => (kind, span.start),
            ParseError::UnexpectedEndOfFile(Expected::Exactly(kind), _, at) => (kind, *at),
            _ => break,
        };
        let close = match kind {
            TokenKind::RightParenthesis => ')',
            TokenKind::RightBracket => ']',
            _ => break,
        };
        if place.is_some_and(|place| place != at) {
            break;
        }
        place = Some(at);
        closers.push(close);
    }
    closers
}

/// The offset just after the last code before `at`, past whitespace and
/// comments.
fn code_end(trivia: &[Trivia<'_>], bytes: &[u8], mut at: usize) -> usize {
    while at > 0 {
        if bytes[at - 1].is_ascii_whitespace() {
            at -= 1;
            continue;
        }
        let covering = trivia.partition_point(|t| (t.span.end.offset as usize) < at);
        match trivia.get(covering) {
            Some(t) if (t.span.start.offset as usize) < at => at = t.span.start.offset as usize,
            _ => break,
        }
    }
    at
}

#[cfg(test)]
mod tests {
    use mago_span::HasSpan;
    use mago_syntax::cst::Statement;

    use super::*;

    /// The text of each top-level statement the parse keeps, with the
    /// brackets the parser found open closed.
    fn kept_statements(content: &str) -> Vec<&str> {
        let arena = LocalArena::new();
        let file_id = mago_database::file::FileId::new(b"input.php");
        let program = parse_file_content(&arena, file_id, content.as_bytes());
        let program = close_open_brackets(&arena, program, content).map_or(program, |(p, _)| p);
        let mut kept = Vec::new();
        for statement in program.statements.iter() {
            let inner: Vec<&Statement<'_>> = match statement {
                Statement::Namespace(ns) => ns.statements().iter().collect(),
                other => vec![other],
            };
            for statement in inner {
                let span = statement.span();
                kept.push(&content[span.start.offset as usize..span.end.offset as usize]);
            }
        }
        kept
    }

    fn keeps_class(content: &str) -> bool {
        kept_statements(content)
            .iter()
            .any(|statement| statement.starts_with("class Shop"))
    }

    fn in_method(body: &str) -> String {
        format!(
            "<?php\nnamespace App;\nuse Lib\\Widget;\nclass Shop {{\n    public function make() {{\n{body}\n    }}\n}}\n"
        )
    }

    #[test]
    fn an_unclosed_call_keeps_the_class_it_is_in() {
        assert!(keeps_class(&in_method("        $w = new Widget(")));
        assert!(keeps_class(&in_method("        $w = new Widget(1, ")));
    }

    #[test]
    fn every_bracket_left_open_is_closed() {
        assert!(keeps_class(&in_method("        foo(bar(")));
        assert!(keeps_class(&in_method("        foo([")));
        assert!(keeps_class(&in_method("        $w = (new Widget(")));
    }

    #[test]
    fn code_after_the_brace_is_kept_too() {
        let content = concat!(
            "<?php\n",
            "class Shop {\n",
            "    public function make() {\n",
            "        $w = new Widget(\n",
            "    }\n",
            "    public function gadget(): Gadget {}\n",
            "}\n",
        );
        assert!(keeps_class(content));
        let content = concat!(
            "<?php\n",
            "function f() {\n",
            "    if ($x) {\n",
            "        foo(\n",
            "    }\n",
            "    $y = 1;\n",
            "}\n",
            "function g() {}\n",
        );
        let kept = kept_statements(content);
        assert!(kept.iter().any(|s| s.starts_with("function f")), "{kept:?}");
        assert!(kept.iter().any(|s| s.starts_with("function g")), "{kept:?}");
        let kept = kept_statements("<?php\nfunction f() {\n    foo(\n}\n");
        assert!(kept.iter().any(|s| s.starts_with("function f")), "{kept:?}");
    }

    #[test]
    fn a_comment_before_the_brace_is_read_past() {
        assert!(keeps_class(&in_method("        foo( // the widget")));
        assert!(keeps_class(&in_method(
            "        foo(\n        /* the widget */"
        )));
    }

    #[test]
    fn each_class_with_an_unclosed_call_is_kept() {
        let content = concat!(
            "<?php\n",
            "class A {\n",
            "    function f() {\n",
            "        foo(\n",
            "    }\n",
            "}\n",
            "class B {\n",
            "    function f() {\n",
            "        bar(\n",
            "    }\n",
            "}\n",
        );
        let kept = kept_statements(content);
        assert!(kept.iter().any(|s| s.starts_with("class A")), "{kept:?}");
        assert!(kept.iter().any(|s| s.starts_with("class B")), "{kept:?}");
    }

    fn keeps_route(content: &str) -> bool {
        kept_statements(content)
            .iter()
            .any(|statement| statement.starts_with("Route::get('/',"))
    }

    fn routes(last: &str) -> String {
        format!(
            "<?php\nnamespace App;\nuse App\\Http\\Controllers\\HomeController;\nRoute::get('/a', fn () => 1);\n{last}"
        )
    }

    #[test]
    fn a_statement_the_end_of_the_file_cuts_off_is_kept() {
        assert!(keeps_route(&routes(
            "Route::get('/', [HomeController::class, 'index']\n"
        )));
        assert!(keeps_route(&routes(
            "Route::get('/', [HomeController::class, 'index'\n"
        )));
        assert!(keeps_route(&routes(
            "Route::get('/', [HomeController::class, 'index'])\n"
        )));
        assert!(keeps_route(&routes(
            "Route::get('/', [HomeController::class, 'index']  // home\n"
        )));
        assert!(keeps_route(
            "<?php\nuse App\\Http\\Controllers\\HomeController;\nRoute::get('/', [HomeController::class, 'index']\n"
        ));
    }

    #[test]
    fn brackets_left_open_at_the_end_of_the_file_are_closed_when_there_is_room() {
        let kept = kept_statements("<?php\nfoo();\n$w = new Widget(\n\n");
        assert!(
            kept.iter().any(|s| s.starts_with("$w = new Widget(")),
            "{kept:?}"
        );
        let kept = kept_statements("<?php\nfoo();\n$w = foo(bar(1\n");
        assert!(
            kept.iter().any(|s| s.starts_with("$w = foo(bar(1")),
            "{kept:?}"
        );
    }

    #[test]
    fn a_file_with_no_room_at_the_end_is_not_reparsed() {
        let arena = LocalArena::new();
        let file_id = mago_database::file::FileId::new(b"input.php");
        let content = routes("Route::get('/', [HomeController::class, 'index']");
        let program = parse_file_content(&arena, file_id, content.as_bytes());
        assert!(close_open_brackets(&arena, program, &content).is_none());
    }

    #[test]
    fn a_function_the_end_of_the_file_cuts_off_is_left_alone() {
        let arena = LocalArena::new();
        let file_id = mago_database::file::FileId::new(b"input.php");
        let content = "<?php\nfunction f() {\n    foo(1\n\n\n\n";
        let program = parse_file_content(&arena, file_id, content.as_bytes());
        assert!(close_open_brackets(&arena, program, content).is_none());
    }

    #[test]
    fn a_closed_call_is_not_reparsed() {
        let arena = LocalArena::new();
        let file_id = mago_database::file::FileId::new(b"input.php");
        let content = in_method("        $w = foo($a");
        let program = parse_file_content(&arena, file_id, content.as_bytes());
        assert!(close_open_brackets(&arena, program, &content).is_none());
    }
}
