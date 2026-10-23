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

### B667. A key read on an intersection of array types resolves nothing

**Impact: Medium · Complexity: Medium**

```php
/** @param array<string, mixed>&array{at: Carbon} $entry */
function f(array $entry): string {
    return $entry['at']->toIso8601String();   // Cannot verify method — type of
}                                             // '$entry['at']' could not be resolved
```

`array<string, mixed>&array{at: Carbon}` is the usual way to say "an
open-ended array that has at least `at`". The intersection itself resolves
and survives spreads, `usort()` and closure-parameter inference from
`array_map()`, but an offset read on it gives no type at all. A read should
take the most specific answer among the members: `Carbon` here, from the
shape. Found in the Laravel sample.

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

### B669. `response($content)` with a nullable argument is typed `ResponseFactory|Response`

**Impact: Medium · Complexity: Low**

```php
public function download(): Response {
    $file = Storage::disk('feeds')->get('feed.xml');   // string|null
    return response($file, 200, $headers);             // reported: ResponseFactory|Response
}
```

The helper's docblock says `($content is null ? ResponseFactory : Response)`,
but its body branches on `func_num_args() === 0`, not on the value: any call
with an argument returns `$factory->make($content ?? '', …)`, a `Response`,
even when that argument is `null`. Following the docblock literally turns
every `response($maybeNull)` into a return-type false positive. The return
type should depend on whether an argument was passed. Found in the Laravel
sample.

## Blade

No outstanding items.

## Templates

### B656. A nested conditional return type tests its inner condition against the unnarrowed parameter

**Impact: Medium · Complexity: Medium-High**

```php
/**
 * @return ($key is Model ? TModel|null : ($key is Arrayable ? static : TModel|null))
 */
public function find(mixed $key): mixed { /* … */ }

function f(Collection $collection, ?User $user): void {
    takesUserOrNull($collection->find($user));
    // reported: got User|Collection<User>|null
}
```

`?User` takes the outer `$key is Model` branch for its `User` part, so the
inner condition only ever sees what the outer one rejected — `null`, which
is not `Arrayable`. We evaluate the inner `$key is Arrayable` against the
whole `?User` instead, `User` matches, and `static` joins a return type it
cannot be part of. Each nested arm has to be resolved against the subject
the enclosing condition left for it, not against the argument as written.
This was PHPStan's own bug until 2.2.16 (phpstan-src#6586); conformance row
`regressions_nested_param_conditional.php`.

## Name resolution

### B657. `new (<expression>)` is not resolved

**Impact: Medium · Complexity: Medium**

```php
$a = new (User::class)();        // unresolved
$b = new ('App\\User')();        // unresolved
$c = new ($factory->class)();    // unresolved, even with @var class-string<User>
$d = new $someClassString();     // resolves
```

PHP 8.0 allows an arbitrary parenthesised expression as the operand of
`new`. The resolver handles the bare-variable spelling (`new $c()`) and
nothing else: with parentheses around the operand the whole `new` resolves
to nothing, so every member access on the instance is an
`unresolved_member_access` false positive. The parenthesised operand should
go through the same class-string resolution the bare variable already uses.
Conformance row `generics_class_string_property_new.php`.

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

### B659. `new $classString` skips the class-kind and constructor checks

**Impact: Low-Medium · Complexity: Low**

```php
new \Countable();                           // reported: Cannot instantiate interface
function f(): void { $c = 'Countable';  new $c(); }   // not reported
function g(): void { $c = '\\Countable'; new $c(); }  // not reported
```

`invalid_class_kind` (and the argument-count check on the constructor) only
runs when `new` names a class syntactically. Once the class comes from a
class-string the checks are skipped entirely, even though the resolver
knows which class it is — including the leading-backslash spelling, which
names the same class as the slash-free one. Run the same checks on the
resolved class whatever syntax named it. Conformance row
`regressions_new_class_string_leading_backslash.php`.

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

### B671. A callback passed by name is not invoked by `call_user_func()` or an in-place `fromCallable()`

**Impact: Low · Complexity: Low**

```php
function f(Repo $r): void {
    call_user_func(callback: [$r, 'find'])->name();          // type is 'mixed'
    \Closure::fromCallable(callback: [$r, 'find'])()->name(); // Method 'name' not found on class 'Closure'
    $c = \Closure::fromCallable(callback: [$r, 'find']);
    $c()->name();                                            // resolves to User
}
```

When the callback is a named argument, the member-access subject path reads
the whole `callback: …` text as the callable and finds no type. The
positional spelling and the assigned `fromCallable()` form both resolve. The
`fromCallable()` shape then falls back to the stub's `Closure` and reports a
false `unknown_member`.

### B673. A partial application held in a variable is checked against the wrapped signature

**Impact: Medium · Complexity: Medium**

```php
/**
 * @template T of object
 * @param class-string<T> $class
 * @return T
 */
public function make(int $id, string $class): object { … }

$c = $r->make(1, ?);
$c(User::class)->name(); // Expected 2 arguments, got 1
                         // Argument 1 ($id) expects int, got class-string<User>
                         // type of '$c(User::class)' could not be resolved
```

Calling a variable that holds a partial application with bound arguments is
checked against every parameter of the wrapped method, so the bound ones count
as missing and the call's arguments are matched to the wrong parameters. The
closure type inferred for `$c` carries no parameters at all, and its return
type is the wrapped call's with no arguments, so a template the placeholder
fills is never bound. The closure's signature should be the wrapped
signature's remaining (placeholder) parameters, and invoking it should combine
the call's arguments with the bound values, the way invoking it in place does.
