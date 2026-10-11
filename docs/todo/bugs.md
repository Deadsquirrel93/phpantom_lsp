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

### B594. `is_subclass_of()` does not narrow a string to `class-string<X>`

**Impact: Medium · Complexity: Low**

```php
function f(string $cls): void {
    if (is_subclass_of($cls, \Countable::class)) {
        takesCountableClass($cls); // got string|class-string
    }
}
```

The narrowed type should be `class-string<Countable>`, as with
`is_a($cls, X::class, true)`. Early-return form
(`if (!is_subclass_of(...)) return;`) and the ternary form
(`return is_subclass_of($c, self::class) ? $c : self::class;`) are
affected the same way.

### B595. A strict `in_array()` narrows the needle to `null`

**Impact: Medium · Complexity: Low**

```php
$in = $parameter['in'] ?? null;
if (in_array($in, ['path', 'query'], true)) {
    takesString($in); // got null
}
```

The needle should become `'path'|'query'`. It comes out as `null`, which
is the one member the haystack rules out. The negated early-`continue`
form gives the same result.

### B596. A comparison with `null` inside a loop leaves the `null` in

**Impact: Low · Complexity: Medium**

```php
/** @param list<array{ts?: string}> $rows */
function f(array $rows, DateTimeImmutable $last): DateTimeImmutable {
    foreach ($rows as $row) {
        $t = isset($row['ts']) ? new DateTimeImmutable($row['ts']) : null;
        $last = $last > $t ? $last : $t;
    }
    return $last; // DateTimeImmutable|null
}
```

An object is always greater than `null`, so the `else` arm never sees a
`null` `$t`. Straight-line code gets this right; only inside the loop does
the result keep `null`.

### B597. A ternary that pairs a value with an instance check is not remembered

**Impact: Low · Complexity: Medium**

```php
$problems = $importer instanceof BrandBibleSheet ? $importer->getProblems() : [];
if ($problems !== []) {
    $this->logProblems($importer, $problems); // expects BrandBibleSheet, got BaseExcelImport
}
```

A non-empty `$problems` implies the `instanceof` arm ran. The stored
conditions already record this kind of implication for `if`/`else`
assignments; the ternary form does not record it.

### B598. `@phpstan-assert` on a property narrows only the object members

**Impact: Low · Complexity: Low**

```php
final class Job { /** @var DateTimeInterface|DateInterval|array|int|null */ public $delay; }

self::assertInstanceOf(DateTimeImmutable::class, $job->delay);
takesInt($job->delay); // got array|int|DateTimeImmutable
self::assertInstanceOf(DateTimeImmutable::class, $local);
takesInt($local);      // got DateTimeImmutable (correct)
```

PHPUnit's `assertInstanceOf()` carries `@phpstan-assert =ExpectedType
$actual`. On a local variable the whole type is replaced; on a property
fetch the `array` and `int` members survive.

### B599. PHPUnit's `assertNotSame([], $x)` does not narrow

**Impact: Low · Complexity: Medium**

```php
preg_match('/…/', $html, $matches);
self::assertNotSame([], $matches);
self::assertStringNotContainsString('x', $matches[0]); // got string|null
```

PHPUnit declares no `@phpstan-assert` on `assertNotSame()`; PHPStan's
PHPUnit extension narrows it (and `assertSame`, `assertNotNull`,
`assertNotEmpty` and others) as if it were the matching `!==`/`===` check.
Mirroring that table would cover the common assertions.

## Array types

### B614. An array literal written inside a call's arguments is typed as bare `array`

**Impact: Low · Complexity: Medium**

```php
takesInt(filter_var($v, FILTER_VALIDATE_INT, ['options' => ['min_range' => 1]]));
// Argument 1 expects int, got mixed (should be int<1, max>|false)
$id = filter_var($v, FILTER_VALIDATE_INT, ['options' => ['min_range' => 1]]);
// int<1, max>|false, as expected
```

A call nested inside another call's arguments is resolved from its argument
text, and that resolver reads any `[…]` or `array(…)` literal as `array`
without looking inside. Anything keyed on an array argument's shape (the
`filter_var()` options, a template bound from a literal array, a
conditional return type on an `array{…}` parameter) loses it there, while
the same call as an assignment's right-hand side goes through the forward
walker and sees the shape. Typing the literal through the shared expression
resolver would make both paths agree.

### B591. `array_map` reads a parenthesised `new` receiver as the element type

**Impact: Low · Complexity: Low**

```php
final class R {
    /** @return list<stdClass> */
    public function all(): array { return []; }
    public function c(): void {
        array_map(fn(stdClass $s) => $s->name, (new R())->all());
        // expects callable(R): mixed, got Closure(stdClass)
    }
}
```

The array argument itself resolves to `list<stdClass>` (passing it to an
`int` parameter says so), and the same call through a variable
(`$x = (new R())->all(); array_map(…, $x)`) is clean, as is `array_filter`
with the same argument. Only the element type `array_map` derives for its
callback's expected parameter lands on `R`, so that derivation appears to
resolve the receiver `(new R())` instead of the whole call.

### B605. Iterating a class constant of enum cases gives `mixed`

**Impact: Low · Complexity: Low**

```php
enum Country: string { case DK = 'dk'; case SE = 'se'; }

final class C {
    private const array TYPED = [Country::DK, Country::SE];
    private const UNTYPED = [Country::DK, Country::SE];

    public function a(): void {
        foreach (self::TYPED as $m) { echo $m->value; }   // $m is mixed
        foreach (self::UNTYPED as $m) { echo $m->value; } // $m is mixed
    }
}
```

The constant's value is a literal list of enum cases, so the element type
is `Country::DK|Country::SE` (or `Country`). Both the typed and the
untyped constant come out as `mixed`.

## Laravel

No outstanding items.

## Blade

No outstanding items.

## Templates

No outstanding items.

## Miscellaneous

No outstanding items.
