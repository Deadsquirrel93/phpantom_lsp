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

### B622. Whether a by-reference callback runs now is read from the current file's text only

**Impact: Low-Medium · Complexity: Medium**

`src/type_engine/variable/forward_walk/by_ref.rs` decides whether a
callback argument is invoked immediately (so its by-reference writes
reach the variables after the call) by re-walking the current file's AST
with `with_parsed_program` and text-scanning the docblock above the callee
for `@param-immediately-invoked-callable` /
`@param-later-invoked-callable`. A callee declared in another file never
has its tag seen and gets the default, and methods are matched by short
class name within the current file. The re-parse also runs per call when
no parse cache is active. The docblock parser already reads both tags
(`docblock/parser.rs`): carry the invocation mode on `ParameterInfo` and
look it up through the function loader and the resolved class.

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

### B618. "Update return type" on a closure writes the enclosing method's type

**Impact: Medium · Complexity: Medium-High**

```php
public function run(): array {
    $f = function (): string {
        return 42;   // PHPStan: Anonymous function should return string but returns int.
    };
    return [];
}
```

The quick fix rewrites the closure to `function (): array` and adds a
`@return array{}` docblock above the statement: the type it writes is the
method's, not the closure's. The return-type quick fixes
(`code_actions/phpstan/fix_return_type/`, `remove_unused_return_type.rs`,
`add_iterable_type.rs`) find their function by scanning text (braces back
from the diagnostic line, the `)` before the `{`, the line holding
`function`), and the inference behind them (`declared_body` in
`fix_return_type/inference.rs`) then looks for a named function or method
around the body brace, so a closure resolves to the method it sits in. An
arrow function has no brace at all, and the text scan lands on the
method's. Locate the function through the parsed AST instead
(`find_enclosing_function` in `code_actions/cursor_context.rs`, with
`FunctionKinds::All`) and read the native return type, the body to infer
from and the docblock (already `paren_function_docblock`) from that one
node.
