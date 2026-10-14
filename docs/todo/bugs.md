# PHPantom — Bug Fixes

Every bug below must be fixed at its root cause. "Detect the
symptom and suppress the diagnostic" is not an acceptable fix.
If the type resolution pipeline produces wrong data, fix the
pipeline so it produces correct data. Downstream consumers
(diagnostics, hover, completion, definition) should never need
to second-guess upstream output.

Each entry below carries an **Impact · Complexity** rating using the same
scale defined in [`docs/todo.md`](../todo.md), but a bug's row lives
**here only** — do not add or link a bug entry to `docs/todo.md`'s sprint
or backlog tables. This file is its own list, not a domain document
sprint items draw from: whenever it holds anything, that is actively
addressed, independently of sprint planning.

Bugs land here from wherever they surface: found while working on another
task, or sweeps of the sample projects under `projects/`. Entries are
grouped by the mechanism that has to change, not by the symptom that
surfaced: one entry is one root cause, however many shapes it shows up in.

## Crashes

No outstanding items.

## Type comparison

No outstanding items.

## Standard-library return types

No outstanding items.

## Reachability

No outstanding items.

## Narrowing

No outstanding items.

## Array types

No outstanding items.

## Laravel

No outstanding items.

## Blade

No outstanding items.

## Templates

No outstanding items.

## Name resolution

No outstanding items.

## Miscellaneous

### B616. PHPStan quick fixes write `\n` line endings into `\r\n` files

**Impact: Low · Complexity: Low**

In a file saved with `\r\n` line endings, the text a PHPStan quick fix
inserts (an `#[Override]` line, an `@throws` or `@return` tag, a whole new
docblock) ends its lines with `\n`, and a docblock the fix rewrites comes
back with all of its lines ending in `\n`. The file is left with mixed line
endings. `sort_use_statements` and `insert_translation_key` already use the
file's own terminator (`content.contains("\r\n")`, each with its own copy
of the check); the actions in `code_actions/phpstan/` and the shared edits
in `code_actions/docblock_edit.rs` write a literal `\n`. One
`line_ending(content)` helper in `text_position.rs` would serve all of
them.

### B617. "Add @throws" is not offered when the method body has a `}` in a string before the throw

**Impact: Low · Complexity: Medium**

```php
public function second(): void {
    $close = '}';
    throw new \RuntimeException();   // no "Add @throws" action
}
```

`find_enclosing_docblock` in `code_actions/phpstan/add_throws.rs` finds the
method by counting braces backwards from the diagnostic line, without
skipping strings or comments. The `}` in the literal throws the count off
by one, the scan settles on the class's `{`, and with no `function`
keyword before it the action is dropped. `find_enclosing_function_line_range`
in the same file already locates the method through the parsed AST; the
docblock lookup should start from that method's span (refactor.md item 2).
