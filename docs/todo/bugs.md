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

### B676. `enum-string` and `trait-string` reject the name of an enum or a trait

**Impact: Medium · Complexity: Low-Medium**

```php
enum SomeEnum { case A; }
trait SomeTrait {}

/** @return enum-string */
function e() { return SomeEnum::class; }   // reported: class-string<SomeEnum> is incompatible with enum-string

/** @param trait-string $t */
function t($t): void {}
t(SomeTrait::class);                        // reported: expects trait-string, got class-string<SomeTrait>
```

Every `X::class` is rejected where an `enum-string` or `trait-string` is
declared, so both spellings are unusable: the valid argument and the
valid return are both errors. Rejecting an ordinary class name there is
right, so the narrowing half works. The problem is that nothing ever
accepts.

`interface-string` has its own `TypeKind` and a class-loader-aware rule
in `class_lookup/subtyping.rs` that loads the inner class of a
`class-string<X>` and accepts it when `X` is an interface. `enum-string`
and `trait-string` are bare named atoms, so `class-string<X>` against
them falls through to `PhpType::subtype_of_symbol_name`, which asks the
name-only lattice `is_named_subtype("class-string", "enum-string")` and
gets a definite `false`. The fix is to give both the same treatment as
`interface-string`: load `X` and compare its `ClassLikeKind` (enum,
trait), stay silent when it cannot be loaded, and accept a bare
`enum-string`/`trait-string` value wherever `class-string`/`string` is
expected as today. A string literal naming an enum or trait needs the
same kind check the literal-to-`interface-string` rule does.

Both rows passed on the run at `5d50cf4cf` and are red again: found
running the php-typing-conformance suite
(`phpdoc_advanced_fallback_enum_string.php`,
`phpdoc_advanced_fallback_trait_string.php`). Every peer LSP measured
(Phpactor, Intelephense, devsense, and for the trait row Qodana) passes
them.

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

### B675. Member references mix up a property and a method of the same name

**Impact: Medium · Complexity: Medium**

```php
class User {
    public function __construct(private string $email) {}
    public function email(): string { return $this->email; }
}
```

Find References on the `$email` property lists the `email()` method's
declaration as well, and renaming the property renames the method too. The
member reference search matches by name and static-ness only: a declaration
span or an access counts whether it is a method or a property, so the
`is_method_call` an access records and the kind of the declaration are
never compared with what was searched for. The search has to carry the kind
of member it is after, and every caller (references, rename, the reference
lens) has to pass it. The same name on both is common on value objects and
Symfony entities, whose validation mappings and form fields name the
property.
