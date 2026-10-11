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

### B582. `keyBy`/`groupBy` on an Eloquent collection lose the element type

**Impact: Medium · Complexity: Medium**

```php
Member::all()->keyBy('id');           // Eloquent\Collection<array-key, mixed>
Member::all()->groupBy('id');         // Eloquent\Collection<array-key, Eloquent\Collection<int, mixed>>
Member::all()->toBase()->keyBy('id'); // Support\Collection<array-key, Member>  (correct)
Member::all()->keyBy->id;             // Eloquent\Collection<array-key, Member>   (correct)
```

`Support\Collection::keyBy` carries only `{@inheritDoc}`; the signature
comes from `Enumerable::keyBy`, which returns
`static<(…conditional key…), TValue>`. `Eloquent\Collection` declares its
own templates (`TKey`, `TModel`) and maps them through
`@extends Support\Collection<TKey, TModel>`. Called on the Eloquent
subclass, the `TValue` inside `static<…>` is not carried through that
mapping to `TModel`, so it ends up unbound and becomes `mixed`. On
`Support\Collection` itself the names line up and it resolves. The
higher-order proxy form builds its result separately and is unaffected.

Any other `static<…, TValue>` return inherited from `Enumerable` through
`{@inheritDoc}` is likely affected the same way.

## Blade

No outstanding items.

## Templates

No outstanding items.

## Miscellaneous

No outstanding items.
