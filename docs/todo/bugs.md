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

### B655. An unknown config key is not reported

**Impact: Low-Medium · Complexity: Low**

```php
config('app.name');       // fine
config('gate.missing');   // not reported
view('missing.page');     // reported: Unknown view
route('gate.missing');    // reported: Unknown route
__('boards.missing');     // reported: Unknown translation key
```

Views, routes and translation keys all have a string-key diagnostic, and
`config()` already resolves far enough to hover a key and name the file it
came from, so the index that would answer this is built. The diagnostic
itself is simply absent. Add it alongside the other three, with the same
conservative stance they take when the index is incomplete. Conformance row
`framework.config-diagnostic` in the language-server probes, the only
non-green framework cell for our column.

## Blade

No outstanding items.

## Templates

No outstanding items.

## Name resolution

### B658. A member access written directly on `new $var` resolves the variable, not the instance

**Impact: High · Complexity: Medium**

```php
function f(Customer $c): void {
    $class = $c::class;
    $copy = new $class();
    $copy->name();                 // resolves to Customer

    echo (new $class())->name();   // Cannot verify method 'name' — type of
                                   // '$class' could not be resolved
}

function g(): void {
    $class = 'App\\Customer';
    echo (new $class())->name();   // Cannot access method 'name' on type
                                   // ''App\Customer''        [scalar_member_access]
}
```

Two resolution paths answer "what does `new $var` produce", and they
disagree. The assignment handler gets it right for every class-string
flavour — `$obj::class`, a literal string, `get_class()`, a
`class-string<T>` docblock. The member-access subject resolver only handles
`class-string<T>`, and only when it is the variable's declared type: a
`class-string<Model>` obtained by `$class = $map[$type] ?? null` and a
`null` guard still fails, with or without the parentheses PHP 8.4 makes
optional. For anything else it either gives up or, worse, hands
back the *operand's* type, so a literal class name is reported as a method
call on a string. That is the parallel-resolution-path anti-pattern:
whichever of the two paths is right, there should be one. Conformance row
`objects_class_constant_on_instance.php`.

## Code actions

No outstanding items.

## Miscellaneous

### B660. A return type is inferred from an untyped body only for methods

**Impact: High · Complexity: Medium**

```php
function makeUser() { return new User(); }
final class Repo { public function find() { return new User(); } }

function f(Repo $r): void {
    makeUser()->name();        // Cannot verify method 'name' — type of
                               // 'makeUser()' could not be resolved
    $r->find()->name();        // resolves to User
}
```

A method with neither a native return type nor `@return` gets its return
type read off its body; a free function with the same shape gets nothing.
Closures and arrow functions do infer, so the method path is the rule and
the free function is the exception. The result is an
`unresolved_member_access` false positive on every call to an unannotated
function — common in legacy code, which is where a language server is worth
the most. Route free functions through the same body inference the method
path uses. Conformance row `native_types_untyped_return_inferred.php`, and
the `inferred-return` hover probe, which shows no type for the same reason.
