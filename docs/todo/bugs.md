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

### B637. An import used only by a call still missing its `)` is reported unused

**Impact: Low · Complexity: Medium**

```php
<?php
namespace App;
use Lib\Widget;   // reported unused until the `)` is typed
class Shop {
    public function make() {
        $w = new Widget(
    }
}
```

mago's argument-list recovery takes the method's `}` for an argument, so the
braces stop balancing: the method body closes on the class's `}`, the class
runs to the end of the file, and the parser drops it whole. Nothing in it
reaches the symbol map, and `collect_unused_import_diagnostics`
(`diagnostics/unused_imports.rs`) counts an import as used only when a span
there names it, so every import the class uses is reported until the call is
closed. A call at the top level of the file's last `namespace` block drops
just its own statement, with the same result. Signature help and
named-argument completion work around the dropped class by re-parsing with
`);` at the cursor (`patch_content_for_signature`), which diagnostics have
no cursor for. The parser's recovery has to end the argument list at the
`}` instead, upstream in `mago-syntax` or by a same-length reparse such as
the one `parser/namespace_recovery.rs` makes for a dropped `namespace`
block.
