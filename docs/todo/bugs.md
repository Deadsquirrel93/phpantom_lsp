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

#### B25. A docblock class name that no class answers to reads as a global class when the file declares a template of that name

**Impact: Low · Complexity: Medium**

```php
namespace App;

/** @template Node */
class Box {}

class Walker {
    public function run(): void {
        /** @var Node $n */
        $n = make();
        $n->missing(); // PHPantom: Method 'missing' not found on class 'Node'
    }
}
```

PHP reads `Node` in the `@var` as `App\Node`. `resolve_source_php_type_names`
(`src/util.rs`) keeps a missed name as written when any docblock in the file
declares it as a `@template` or type alias, since it may be one in scope
there. The resolved loader then reads the bare `Node` as the global class.
Checking the whole file rather than the declarations in scope is what lets
`Box`'s template reach `Walker`.

**Where to change:** decide template and alias names from the declarations in
scope at the name (the enclosing class's and function's `@template` tags and
the class's aliases), and qualify every other miss with the namespace as
`resolve_code_class_name` does.

#### B28. A `use function` or `use const` import can claim a class name

**Impact: Low-Medium · Complexity: Medium**

```php
namespace App\Console;

use function Laravel\Prompts\table;

class Table {}

$t = new Table(); // PHPantom: `Laravel\Prompts\table`, not `App\Console\Table`
```

The file's `use` map holds function and constant imports under their short
name alongside class imports (`register_use_item` in
`src/parser/use_statements.rs`), and class-name resolution cannot tell them
apart. Since class imports match case-insensitively (`lookup_import` in
`src/util.rs`), `Table` finds the `table` function import, and an import that
claims a name is final, so the class is never looked up in the namespace. A
`use const App\VERSION;` does the same to a class `Version`. When two keys
differ only in case, which one answers depends on hash-map order. Other
readers of the map treat every entry as a class too (class completion,
`use_map.values()` in `completion/handler/class_constant.rs`).

The same split leaves declarations case-sensitive: `resolve_name` in
`src/parser/ast_update.rs` matches imports exactly, so with `use Lib\Money;`
a hint `money $m` reads as `App\money` while `new money()` in a body reads as
`Lib\Money`. Making it case-insensitive before function imports are split out
would spread the collision above to every signature.

**Where to change:** keep function and constant imports out of the class
import table (a separate map per kind, as `mago-names` resolves them), then
switch `resolve_name` to `lookup_import`.

## Miscellaneous

No outstanding items.
