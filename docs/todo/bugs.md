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

#### B552. A by-reference closure capture keeps the narrowing from where the closure was created

**Impact: Low · Complexity: Medium**

```php
if ($c !== null) {
    $f = function () use (&$c) { take($c); };   // `$c` reads as non-null
    $c = null;
    $f();                                        // TypeError at runtime, not reported
}
```

A by-value capture is copied when the closure is created, so seeding it from
the scope at that point is right. A by-reference capture is read when the
closure runs, and the variable can be reassigned in between, so the
narrowing at the creation point does not hold inside the body.
`seed_closure_captures` seeds both kinds the same way, in an `if` block as in
a ternary, `&&` or `match` branch. A by-reference capture should also
carry every type the enclosing function assigns to the variable after the
closure is created.

#### B27. An offset on a receiver of an unknown class is not narrowed

**Impact: Low · Complexity: Low**

```php
namespace App;

/** @param ArrayAccess<int, ?object> $array */
function run(ArrayAccess $array): void {
    assert($array[1] === null);
    // PHPStan: `$array[1]` is `null`. PHPantom: still `?object`.
}
```

Unimported, `ArrayAccess` names `App\ArrayAccess`, which does not exist. An
`assert()` or `if` on an offset of a receiver whose class is unknown narrows
nothing, though the check says as much about the offset whatever the receiver
is. PHPStan's `bug-13214.php` in `tests/phpstan_nsrt/` is skipped for this.

**Where to change:** the offset narrowing in
`src/type_engine/variable/forward_walk/cond_narrowing/` should key the
narrowed offset by expression even when the receiver resolves to no class.

#### B24. An `instanceof` branch leaves its class behind after the `if`

**Impact: Medium · Complexity: Medium**

```php
namespace App;

class Node { public function nodeOnly(): void {} }

function run(object $o): void {
    if ($o instanceof Node) {
        $o->nodeOnly();
    }
    $o->anything(); // PHPantom: Method 'anything' not found on class 'App\Node'
}
```

After the `if`, `$o` is the join of the branch that narrowed it to `Node` and
the path that skipped the branch, where it is still `object`. `object|Node` is
just `object`, which PHPantom does not check member access on, but the join
keeps `Node` as a class of its own and the member check reports what `Node`
lacks. A union where one interface member covers a narrowed subclass
(`Countable` joined with `ArrayObject`) may show the same thing; not yet
checked.

**Where to change:** the branch merge in the forward walker
(`src/type_engine/variable/forward_walk/`) should drop a class the other
branch's type already contains, or the member check should treat a union
member of `object` as covering the rest. Check which consumer reads the join
before deciding.

## Array types

No outstanding items.

## Laravel

No outstanding items.

## Blade

No outstanding items.

## Templates

#### B23. A chained call drops a conditional return that tests a template bound by an omitted argument

**Impact: Medium · Complexity: Medium**

```php
interface ContainerInterface
{
    /**
     * @template C of object
     * @template B of int
     * @param string|class-string<C> $id
     * @param B $invalidBehavior
     * @return ($id is class-string<C> ? (B is 0|1 ? C : C|null) : object)
     */
    public function get(string $id, int $invalidBehavior = 1): ?object;
}

function run(ContainerInterface $c): void {
    $svc = $c->get(Svc::class);
    $svc->missing();                // reported against Svc
    $c->get(Svc::class)->missing(); // PHPantom: not reported, the type is lost
}
```

Assigned to a variable, the call resolves to `Svc`; called inline, it
resolves to nothing, whether the omitted argument's default is a literal or a
class constant. With the inner `B is 0|1` condition removed, both forms
resolve. Symfony's `ContainerInterface::get()` has this shape, so
`$container->get(Foo::class)->bar()` loses `Foo`.

The cause is not pinned down yet. The assigned form resolves through the
forward walker (`resolve_owner_method_call` in
`src/type_engine/variable/rhs_resolution/calls.rs`) and the inline one through
`resolve_method_return_types_with_args`
(`src/type_engine/call_resolution/return_types.rs`). A `B is …` condition is
decided by the argument that binds `B`, which
`resolve_conditional_with_text_args_and_defaults`
(`src/type_engine/types/conditional.rs`) finds through `tpl.bindings`, so a
first thing to check is whether the inline path hands it the method's
template bindings.

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

#### B26. A `key-of`/`value-of` operand in a callee's docblock is read through the caller's imports

**Impact: Low · Complexity: Medium**

```php
namespace {
    enum Suit: string { case Hearts = 'h'; }
    /** @param value-of<Suit> $s */
    function take(string $s): void {}
}
namespace App {
    enum Suit: string { case Other = 'x'; }
    function run(): void {
        take('h'); // PHPantom: Argument 1 ($s) expects 'x', got 'h'
    }
}
```

`constant_operand_shape` and `enum_backing_values`
(`src/type_engine/call_resolution/template_subs.rs`) resolve the class an
operand names with the call site's source loader and namespace. Some operands
arrive already qualified by the declaring file and some as written (a
`key-of<Foo::BAR>` class constant is not qualified at parse time), so neither
loader is right for all of them. Switching both to the resolved loader fixes
the case above but breaks the `key-of` over class constant tests.

**Where to change:** qualify the class names inside type-operator operands
when the declaring file is parsed, as other docblock types are, then read
them with the resolved loader.

## Miscellaneous

No outstanding items.
