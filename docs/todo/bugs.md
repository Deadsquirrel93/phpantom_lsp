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

## Code actions

No outstanding items.

## Miscellaneous

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
