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

### B638. A monorepo's subprojects miss path repositories and are never rescanned after `composer update`

**Impact: Low-Medium · Complexity: Medium**

A workspace with no root `composer.json` goes through `init_monorepo`
(`indexing/init.rs`), which sets up each subproject with its own copy of the
Composer steps. That copy leaves out the PSR-4 mappings of path-repository
packages (`composer::extract_path_repo_psr4_mappings`), which
`init_autoload_paths` adds for a single project, so a class in a local
module that the classmap does not list yet never resolves. After startup a
`composer.json` or `composer.lock` change runs `rescan_composer_indexes`
(`indexing/scan.rs`) on the workspace root only. It reads the root's
`composer.json`, finds none, and just clears the caches, so a subproject's
vendor classes, autoload functions and constants stay as they were at
startup until a restart. The copy also never classifies the classes the
subproject's Composer classmap lists, which a single project does with
`classify_class_origin`, so where an optimized autoloader lists every
vendor class, completion ranks all of them as project code. Run the
single-project Composer setup per subproject root, and rescan the
subproject whose Composer files changed.

### B639. A `Pest.php` changed outside the editor is never re-read

**Impact: Low · Complexity: Low**

The `->in(…)` bindings a test file's closures are typed from come from the
`Pest.php` files above it, cached per directory in
`Backend::pest_configs` (`pest.rs`), including the directories found to
have none. The cache is only dropped when a file ending in `/Pest.php` is
saved in the editor (`on_did_save`, `backend/documents.rs`). One created by
`pest --init`, edited by another tool, or changed by a branch switch reaches
the server as a watched-file event, which `apply_watched_file_changes`
(`indexing/watch.rs`) re-indexes without touching that cache, so test
closures keep the old binding, or none, until a restart. Invalidate it there
as well.

### B640. Two edits count columns in bytes, so they land too far right after non-ASCII text

**Impact: Low · Complexity: Low**

```php
/** @var object{größe: int, name: string} $item */
echo $item->name;
```

An LSP column counts UTF-16 code units, but two places still turn a byte
offset into a column by hand and report the byte count. Any non-ASCII
character earlier on the line pushes the result right by its extra bytes:
go-to-definition on `name` above lands two columns into the key instead of
on its first letter. The hand-rolled conversions are object-shape key
go-to-definition (`definition/member/shape_keys.rs`, which returns the
key's byte index within its line as the `character`) and the ranges
"Remove assert" deletes (`code_actions/phpstan/remove_assert.rs`). Convert
an absolute byte offset through `offset_to_position` (`text_position.rs`)
instead.

### B641. "Generate promoted constructor" orphans each property's `@var` docblock

**Impact: Low · Complexity: Low**

```php
class Order {
    /** @var list<Item> */
    private array $items;
    public static int $count = 0;
}
```

The action deletes each property's declaration and re-declares it as a
promoted parameter, but the deleted span (`declaration_span` in
`generate_constructor.rs`, from `find_line_start` of the member) stops at
the declaration, so the `/** @var list<Item> */` above it stays. It then
documents whatever follows, here the static `$count`, whose type it now
claims is `list<Item>`. "Promote constructor parameter" deletes the docblock
with the property (`docblock_start_above_offset`,
`promote_constructor_param.rs`); this action should too.
