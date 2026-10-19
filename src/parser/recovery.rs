//! Parsing again the code mago's parser drops while the file is being
//! edited.
//!
//! Each step rewrites a few bytes of the text without moving any offset,
//! so the reparse describes the text the caller holds:
//! [`bracket_recovery`](super::bracket_recovery) closes the brackets an
//! unfinished expression leaves open before a `}` or at the end of the
//! file, and [`namespace_recovery`](super::namespace_recovery) opens
//! unbraced the braced `namespace` blocks that still run to the end of the
//! file.

use mago_allocator::LocalArena;
use mago_syntax::cst::Program;
use mago_syntax::parser::parse_file_content;

/// Parse `content`, keeping the code the parser would drop (see
/// [`reparse_dropped_code`]).
pub(crate) fn parse_program<'a>(arena: &'a LocalArena, content: &'a str) -> &'a Program<'a> {
    let file_id = mago_database::file::FileId::new(b"input.php");
    let program = parse_file_content(arena, file_id, content.as_bytes());
    reparse_dropped_code(arena, program, content).unwrap_or(program)
}

/// Parse `content` again keeping the code its parse, `program`, dropped,
/// or `None` when it dropped none it could keep.
///
/// The reparse and the text it reads are allocated in `arena`.  Its
/// `errors` describe that text, so a caller reporting syntax errors takes
/// them from `program`.
pub(crate) fn reparse_dropped_code<'a>(
    arena: &'a LocalArena,
    program: &'a Program<'a>,
    content: &str,
) -> Option<&'a Program<'a>> {
    // A file that parsed without complaint dropped nothing.
    if program.errors.is_empty() {
        return None;
    }
    let closed = super::bracket_recovery::close_open_brackets(arena, program, content);
    let (parse, text) = closed.map_or((program, content), |(parse, text)| (parse, text));
    super::namespace_recovery::reparse_dropped_namespace_blocks(arena, parse, text)
        .or(closed.map(|(parse, _)| parse))
}
