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

### B686. A scalar passed to a `bool` parameter is reported outside `strict_types`

**Impact: Low-Medium · Complexity: Low**

```php
function flag(bool $on): void {}

flag(1);           // Argument 1 ($on) expects bool, got 1
flag(0.5);
flag('yes');
microtime(1);
```

Without `declare(strict_types=1)`, PHP converts an `int`, `float`, or
`string` argument to `bool`, so all four calls run. The non-strict
coercions in `src/class_lookup/compatibility.rs` cover int/float to
string, numeric string to int/float, and float to int, but not
scalar to bool. Found in a legacy codebase where `microtime(1)` alone
accounted for 47 reports.

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

### B684. A class one Composer project installs resolves in every other project of the workspace

**Impact: Medium · Complexity: High**

```text
repo/
├── composer.json
├── platform/            ← requires guzzlehttp/guzzle
│   ├── composer.json
│   └── vendor/
└── mobile/              ← does not
    ├── composer.json
    └── vendor/
```

A file under `mobile/` that names `GuzzleHttp\Client` gets completion,
hover and no `unknown_class`, although running it would fail: the
class only exists in `platform/vendor/`. Every Composer project of a
workspace (a monorepo's subprojects, or projects nested below a root
`composer.json`) feeds one shared class index, so a lookup does not
know which project the file asking belongs to. Two projects installing
different versions of one package also share whichever copy was
indexed first.

Lookups from a file need to go through the index of the project that
holds it, falling back to the shared one only for the project's own
sources and the workspace's loose files.

**Where to look:** `ComposerProjectPaths` in `src/workspace_env.rs`
(each project's vendor and PSR-4 mappings are already recorded), and
the class lookup in `src/resolution/`.

### B685. A same-named class in a parsed file hides what the real class inherits

**Impact: High · Complexity: Medium**

```text
core/base.php        namespace Lib; abstract class Base { public static function getList(): array { return []; } }
core/store.php       namespace Lib; class Store extends Base {}
local/tests/stub.php namespace Lib; class Store {}
local/app/use.php    \Lib\Store::getList();
```

`analyze local/` reports `Method 'getList' not found on class
'Lib\Store'`. `analyze local/app/` and `analyze .` report nothing, and
neither does the same layout when `getList()` is declared on `Store`
itself rather than inherited. So when the stub is parsed and the real
declaration is known only from the scan, the name resolves to a class
that has the real one's own members but not its parent chain, and which
declaration wins depends on which files happen to be parsed rather than
on the "lowest-sorting URI wins" rule the parse phase applies between
two parsed declarations. In an editor, opening the stub's file is
enough.

Test bootstraps that declare empty doubles of framework classes behind
`if (!class_exists(...))` are where this shows up: one Bitrix project
lost `getList()`, `add()`, and `update()` on every core entity a test
doubled, about 1,280 diagnostics.

**Where to look:** the duplicate-declaration handling in
`src/parser/ast_publish.rs` (`decls.declare`) and how a class known
only from the scan is loaded against an already-parsed one of the same
FQN.

## Code actions

No outstanding items.

## Miscellaneous

No outstanding items.
