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

### B682. A templated override keeps the parent's wider `@return` beside its own native type

**Impact: Low-Medium · Complexity: Medium**

```php
interface View { public function name(): string; }
interface Htmlable {}

abstract class Component {
    /** @return View|Htmlable|\Closure|string */
    abstract public function render();
}

/** @template TModel */
final class PerPage extends Component {
    public function render(): View { throw new Exception(); }
}

function f(PerPage $p): void {
    $p->render()->name();   // resolves on View|Htmlable|Closure
}
```

`render()` declares `: View` and has no docblock, so its return type is
`View`. It resolves to `View|Htmlable|Closure` instead: the inheritance
merge takes the parent's `@return` and
`without_alternatives_the_native_type_forbids` (in
`src/php_type/normalize.rs`) only drops alternatives whose value domain
the native type rules out, so `string` goes and every class stays. The
same override without `@template TModel` on its class resolves to `View`,
so a second path, taken only for a templated class, is the one that
lets the wider type through. PHPStan discards an inherited `@return` that
is not a subtype of the native type.

Found running the opt-in `partial-union-members` diagnostic over a
Laravel project: a Blade component's `render(): View` reported `name()`
as missing on `Htmlable` and `Closure`.

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

## Code actions

No outstanding items.

## Miscellaneous

No outstanding items.
