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

### B630. An unclosed call inside a braced `namespace` block drops the block

**Impact: Low · Complexity: Medium**

```php
<?php
namespace Lib {
    function greet(string $name): void {}
}
namespace App {
    use function Lib\greet;
    greet(   // until the `)` is typed, this line reads as `namespace Lib`
}
```

While a call inside a braced `namespace App { … }` block is missing its
`)`, mago's parser leaves the whole block out of `program.statements`, so
`update_ast` (`parser/ast_update.rs`) records no `NamespaceSpan` for it and
indexes none of its imports, classes or functions. An offset in it falls
into the last block the parse kept (`NamespaceSpan::containing`), and
every name typed there resolves against that block's namespace and imports
until the call is closed. Signature help runs on exactly this text, so it
shows the wrong function or none. A file that declares its blocks as
`namespace App;` keeps every block. The block survives only if the
parser's recovery ends the argument list at the block's `}` instead of
dropping the statement: upstream in `mago-syntax`, or by re-parsing with
the call closed, as signature help's text fallback already does for the
classes (`patch_content_for_signature`).

### B633. Diagnostics forget top-level variables at a braced `namespace` block

**Impact: Low · Complexity: Low**

```php
<?php
namespace A {
    $x = new \DateTime();
    $x->nope();   // reported: no `nope()` on DateTime
}
namespace B {
    $x->nope();   // not reported, though `$x` is still the DateTime
}
```

PHP keeps one global variable scope across a file's `namespace` blocks, and
hover follows it: `walk_body_forward` (`forward_walk/statements.rs`) walks
each block's statements on the same scope, so hovering `$x` in block B shows
`DateTime`. The diagnostic walk does not: `walk_top_level_statements`
(`forward_walk/diagnostic_walk.rs`) recurses into each `Statement::Namespace`
and starts it on a fresh `ScopeState`, so a variable assigned in one block
is untyped in the next and nothing checks the members, arguments or types
it is used with there. Carry the top-level scope from block to block,
switching only the loaders and imports, as the recursion's `block_ctx`
already does.

### B634. Edits outside the PHPStan quick fixes write `\n` line endings into `\r\n` files

**Impact: Low · Complexity: Medium**

In a file saved with `\r\n` line endings, most edits other than the PHPStan
quick fixes end the lines they add with `\n`: the `use` line an import adds
(from completion, "Import class", or the `use Override;` of "Add
`#[Override]`"), a generated constructor, getter, setter or property hook,
implemented methods, an extracted function, constant or interface, "Update
docblock", the docblock generated on `/**`, and an override completion,
among others. The file is left with mixed line endings. Each builder writes a literal
`\n` (`build_constructor` in `generate_constructor.rs`,
`extract_function/codegen.rs`, `update_docblock/rewrite.rs`, …) where it
should write `line_ending(content)` from `text_position.rs`, as the PHPStan
quick fixes and `sort_use_statements` do. Imports need the terminator
carried in: `UseBlockInfo::import_text` (`completion/use_edit.rs`) and the
template's `@use` writer (`blade/use_block.rs`) build their line without
the file's content, so `analyze_use_block_in` and
`analyze_template_use_block` would record it in the `UseBlockInfo`, and
`drop_repeated_separator` would strip that terminator instead of `\n`.

### B635. "Remove return statement" moves the `return` out of a braceless `if`

**Impact: Low · Complexity: Low**

```php
public function run(): void {
    if ($this->ready())
        return $this->save();   // PHPStan: ... returns bool but should not return anything.
    $this->log();
}
```

The quick fix keeps the value as a statement and adds a `return;` on the
line after it, so the method still stops there. When the `return` is the
only statement of an `if`, `else`, `elseif` or a loop written without
braces, that `return;` lands after the construct instead of inside it: the
method above now returns before `$this->log()` every time. On one line,
`if ($x) return $v;` becomes `if ($x) $v;` and `if ($x) return;`, which
evaluates the condition twice. `build_strip_return_expr_edit`
(`code_actions/phpstan/fix_return_type/edits.rs`) writes the two
statements without looking at what holds the `return`; when that is a
braceless body, they need braces around them. `find_target`
(`fix_return_type/target.rs`) already walks down to the statement and can
report what holds it.

### B636. A docblock that mentions "function" loses its inferred `@return`

**Impact: Low · Complexity: Low**

```php
class Foo {
    /**
     * Builds the function map. {@see Bar}
     */
    public function ids(): array {
        return [1, 2];
    }
}
```

"Update docblock" adds no `@return array{1, 2}` here, as it would under
another summary. `enrichment_return_type`
(`code_actions/phpstan/fix_return_type/inference.rs`) finds the function
from the docblock's position by text: the first line holding `function `
within ten lines, then the first `{` within six lines of that. The summary
line matches both, so the inference reads from an offset inside the
docblock, where no function is declared. Both callers know the declaration
they are documenting: `analyse_body` (`update_docblock/changes.rs`) built
its `FunctionWithDocblock` from the method's node, and `build_docblock`
(`completion/phpdoc/generation/build.rs`) has the declaration text right
after the trigger. Pass the declaration's offset in and let
`infer_return_type_at` find the function there.
