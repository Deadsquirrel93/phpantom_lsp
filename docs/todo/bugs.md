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

### B679. A class template in `@return` gives way to a scalar or nullable native return type

**Impact: Medium · Complexity: Medium**

```php
/** @template TInput of string|int|null */
final class InputBag
{
    /** @return TInput */
    public function first(): string|int|null {}
}

/** @var InputBag<string> $query */
$query->first(); // string|int|null, should be string
```

A method's `@return` naming a template of its class is judged against the
native return type as if the template were a class name: a native type
made of scalars rejects it, so the method returns its native type whatever
the class is instantiated with. A nullable native type lets it through but
adds its `null`, so `@return TParent` on `end(): ?NodeParentInterface`
returns `?TreeBuilder<'array'>` where the binding is a `TreeBuilder<'array'>`.
A template of the method itself is compared by its bound
(`resolve_effective_type_with_template_bounds`), but the method's return
type is resolved in `parser/signature.rs` before any bounds are known, and
the class's bounds never reach it. The bound comparison has its own slip:
it strips `null` from the docblock type before substituting the bound, so a
bound that includes `null` (`TInput of string|int|null`) is never a subtype
of the native type stripped the same way.

Symfony 7.4 declares `InputBag`, `Request::$query`, and the whole config
tree builder this way, so `$request->query->get('page')` and every
`->end()` of a tree builder come out wrong. The fixtures marked
`// SKIP: a class template in @return gives way to a scalar or nullable
native return type` in `tests/phpstan_symfony/data/` show each case.

## Name resolution

### B677. `unset` in a `@var` union is resolved as a class name

**Impact: Medium · Complexity: Low-Medium**

```php
/** @var \DateTime|unset $guarded */          // reported: Class 'App\unset' not found
if (isset($guarded)) {
    echo date_format($guarded, 'Y-m-d');      // reported: App\unset does not satisfy DateTimeInterface
}
```

`unset` in a docblock union is the Blade-view idiom for "this variable
may not be defined at all" (issue #366). We resolve the spelling against
the current namespace like any class name, so every such `@var` is an
`unknown_class`, and the phantom class then rides along in the variable's
type: `isset()` does not remove it, so the guarded call still reports an
argument mismatch.

`unset` has to be a docblock keyword (`php_type/keywords.rs`) rather than
a class name, and the type it stands for has to be something `isset()`
narrows away. The simplest honest reading is `null`: `\DateTime|unset`
becomes `?\DateTime`, which the existing `isset()`/`!== null` narrowing
already handles. A true possibly-undefined state would be more precise,
but nothing downstream needs it to make the guarded reads correct.

Found running the php-typing-conformance suite
(`regressions_unset_pseudo_type.php`). Phpactor, Intelephense, devsense
and Qodana all pass it.

## Code actions

No outstanding items.

## Miscellaneous

### B678. An inline `{@see Class::$prop}` in a docblock's prose is not a reference

**Impact: Low-Medium · Complexity: Low-Medium**

```php
class Docblock {
    public static $vectors = [];

    /**
     * The tags defined in the docblock.
     *
     * The array has keys which are the tag names (excluding the @) and values
     * that are arrays, each of which is an entry for the tag.
     *
     * In the case where the tag name is defined in {@see DocBlock::$vectors} the
     * value within the tag-value array is an array in itself with keys as
     * described by {@see DocBlock::$vectors}.
     *
     * @var array
     */
    public $tags;
}
```

Find References on `$vectors` returns a standalone `@see DocBlock::$vectors`
tag in another file, but neither inline `{@see DocBlock::$vectors}` above,
so renaming the property leaves both behind. `scan_text_for_inline_see` in
`src/symbol_map/docblock.rs` is meant to cover free text ahead of the first
tag and the description of any tag. Not yet confirmed: the bare `@` in
"(excluding the @)" may start a tag as far as the PHPDoc grammar is
concerned, so the following prose would land in a tag value that is never
scanned. A reproduction with and without that `@` will tell.

Found re-measuring the php-typing-conformance LSP probes (psysh navigation
corpus, `Psy\Util\Docblock::$vectors`). That table counts code references
only, so it scores the `@see` tag we do return as an extra. Docblock
references are deliberate (rename has to update them), so the fix here is
to find the inline ones too, not to drop the tag.

### B680. A `@param-out` wider than the argument keeps the argument's type

**Impact: Low-Medium · Complexity: Low**

```php
/** @param-out array $x */
function fill(array &$x): void {}

$values = [1 => 'a'];
fill($values);
$values; // array{1: 'a'}, should be array
```

After a by-reference call, the argument holds whatever the callee wrote,
which `@param-out` declares. The by-reference seeding in
`type_engine/variable/forward_walk/by_ref_builtins.rs` keeps the argument's
type from before the call when the parameter's type covers it, which is
right for a native hint that says nothing about what the callee writes
(`array_shift(array &$array)` keeps a `Node[]` a `Node[]`). It does the
same for an explicit `@param-out`, the author's statement of what is
written, so the argument keeps its old shape, literals included. Symfony's
`PropertyAccessor::setValue()` declares `@param-out` this way
(`tests/phpstan_symfony/data/property_accessor.php`). An explicit
`@param-out` should replace the argument's type.
