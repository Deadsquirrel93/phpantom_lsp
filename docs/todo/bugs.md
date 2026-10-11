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

### B592. `class_implements()` keeps `false` for a class known to exist

**Impact: Low · Complexity: Low**

```php
if (is_string($c) && class_exists($c) && in_array(Foo::class, class_implements($c), true)) {}
// Argument 2 ($haystack) expects array, got array<string>|false
```

`class_implements()`, `class_parents()` and `class_uses()` only return
`false` when the class cannot be loaded. After a `class_exists()` check
(or on a literal `Foo::class`) the `false` cannot happen. PHPStan returns a
benevolent `array|false` here, which is why it stays quiet.

### B593. `filter_var()` returns `mixed` whatever the filter

**Impact: Low · Complexity: Low**

```php
$id = filter_var($v, FILTER_VALIDATE_INT, ['options' => ['min_range' => 1]]);
// PHPantom: mixed. PHPStan: int<1, max>|false
```

`FILTER_VALIDATE_INT`, `FILTER_VALIDATE_FLOAT`, `FILTER_VALIDATE_BOOL` and
friends decide the result type (plus `false`, or `null` with
`FILTER_NULL_ON_FAILURE`). With `mixed`, `$texts[$id] = …` widens a
documented `array<int, string>` to `int|string` keys, which then fails the
array's own `@var`.

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

### B590. A facade `@method` tag that drops generic arguments hides the accessor's return

**Impact: Medium · Complexity: Low**

```php
use Illuminate\Http\Client\Factory;
use Illuminate\Support\Facades\Http;

Http::recorded();          // Collection
(new Factory)->recorded(); // Collection<int, array{0: Request, 1: Response|null}>

Http::recorded(fn (Request $r) => true)
    ->map(fn (array $pair) => $pair[0]->url()); // cannot verify 'url'
```

`Http`'s generated tag is `@method static Collection recorded(…)`, while
`Factory::recorded()` declares the shape. `facade_concrete_owner`
(`call_resolution/facade_owner.rs`) only falls through to the
`getFacadeAccessor()` class when the concrete return is call-dependent (a
conditional or a method template), so a tag that merely flattened
`Collection<…>` to `Collection` wins.

**Fix:** Also fall through when the concrete method's return carries type
arguments or a shape the facade tag's return lacks, for the same bare class.

### B606. A `when()` callback is checked against the untruthy value type

**Impact: Low · Complexity: Medium**

```php
/** @var ?string $jobClass */
$query->when(
    $jobClass,
    fn (Builder $query, string $jobClass) => $query->where('x', $jobClass),
);
// expects callable($this, ?string): $this, got Closure(Builder, string)
```

`Conditionable::when()` only calls the callback when the value is truthy
(and `unless()` only when it is falsy), so the callback's parameter can
never see `null` here. The template `TWhenParameter` is bound to the raw
`?string`. Removing the falsy members (or only the truthy ones, for
`unless()`) from the binding before checking the callback would match the
runtime.

### B607. `View::addNamespace()` in a provider does not register the namespace

**Impact: Medium · Complexity: Low**

```php
final class ViewServiceProvider extends ServiceProvider {
    public function boot(): void {
        View::addNamespace('webshop', base_path('resources/webshop/views'));
    }
}
```

```blade
@include('webshop::components.box') {{-- Unknown view: 'webshop::components.box' --}}
```

`provider_resources.rs` records `loadViewsFrom($path, $namespace)` but not
the `View` facade's `addNamespace()` (nor `$this->app['view']->addNamespace()`
or `View::prependNamespace()`). One large Laravel project registers all
three of its view namespaces this way, which leaves every
`view('webshop::…')` and `@include('backoffice::…')` unknown.

## Blade

### B608. A class component outside `App\View\Components` is not linked to its template

**Impact: High · Complexity: Medium**

```php
namespace App\Contexts\Webshop\View\Components\Products;

final class Slider extends Component {
    public readonly string $declared;
    public function __construct(public readonly int $promoted, public bool $flag = false) {
        $this->declared = 'x';
    }
    public function modifier(): string { return ''; }
    public function render(): View { return view('components.products.slider'); }
}
```

```blade
{{-- resources/webshop/views/components/products/slider.blade.php --}}
<div class="{{ $declared }} {{ $promoted }} {{ $flag }} {{ $modifier() }}"></div>
{{-- Undefined variable for all four --}}
```

The same class under `App\View\Components\Slider` with the template at
`resources/views/components/slider.blade.php` is clean, so the template is
only linked to its class by Laravel's default naming convention, not by
the view the class's `render()` returns or by `Blade::componentNamespace()`.
When the component is used as a tag, call-site attribute inference covers
the variables passed as attributes, so only defaults, computed properties
and public methods show up as undefined. One large Laravel project has 55
of these across its webshop components. A Livewire component's view
(`$this->method()` inside `components.customers.add-test-order`) shows the
same symptom and is likely the same gap.

### B609. A class component's `render()` view is reported as missing `$attributes`

**Impact: Low · Complexity: Low**

```php
public function render(): View {
    return view('components.products.ean-code', ['value' => $this->raw]);
}
```

```blade
@php /** @var \Illuminate\View\ComponentAttributeBag $attributes */ @endphp
{{-- View 'components.products.ean-code' expects $attributes … which is not passed --}}
```

Laravel adds `attributes`, `slot` and `componentName` (plus the public
properties and methods) to the data of a view a class component renders,
so the template can always rely on them.

### B610. A Blade `@include` does not count as reading the parent's variables

**Impact: Medium · Complexity: Low**

```blade
@if ($flag)
    @php $spanClass = 'a'; @endphp
@else
    @php $spanClass = ''; @endphp
@endif

@include('child') {{-- child.blade.php reads $spanClass --}}
{{-- Unused variable '$spanClass' --}}
```

Laravel compiles `@include` (and `@includeIf`, `@includeWhen`,
`@includeUnless`, `@includeFirst`, and `@extends`) to pass
`get_defined_vars()` to the child, so every variable in scope is read. A
plain PHP `include` already exempts its frame in `unused_variables.rs`
(`frame_owns_import`); the Blade directives produce neither an `include`
nor a `get_defined_vars()` call, so the variable counts as unused. Only the
first write is reported, which is why one branch looks fine.

## Templates

### B585. A class named like a template parameter shadows the template

**Impact: Medium · Complexity: Low**

```php
// anywhere in the index, e.g. a test fixture
trait T {}
```

```php
namespace App;

interface Node { public function getId(): string; }

/** @template T of Node */
final class Items {
    /** @param T[] $items */
    public function __construct(array $items) {
        foreach ($items as $item) {
            $item->getId(); // Method 'getId' not found on class 'T'
        }
    }
}

final class D {
    public function run(string $dir): void {
        new \RecursiveIteratorIterator(new \RecursiveDirectoryIterator($dir));
        // Argument 1 ($iterator) expects T, got RecursiveDirectoryIterator
    }
}
```

With a class-like named `T` anywhere in the project, every `T` template
(class-level, method-level, inherited through an interface, and the stubs'
own) resolves to that class instead of the template parameter. Removing
the trait makes all of them clean. Template names in scope have to win
over class lookup. The case above also resolves an unqualified `T`
inside `namespace App` to the global `\T`, which PHP never does for class
names, so that fallback may deserve its own look.

### B587. A literal constructor argument binds its class template as `string`

**Impact: Medium · Complexity: Medium**

```php
/** @template T of 'array'|'string' = 'array' */
final class TB {
    /** @param T $type */
    public function __construct(string $name, string $type = 'array') {}

    /** @return (T is 'array' ? ArrNode : StrNode) */
    public function root(): object { return new ArrNode(); }
}

(new TB('x', 'array'))->root()->children(); // not found on class 'StrNode'
```

The `'array'` argument binds `T` as `string` rather than the literal, so
the conditional return takes the wrong branch. Symfony 8's config
`TreeBuilder` uses exactly this shape (`new TreeBuilder('name', 'array')`
then `getRootNode()->children()`), which makes every config-tree builder
on Symfony 8 report `StringNodeDefinition` and then cascade into
`unresolved_member_access` hints down the chain.

### B600. A `$callback is Closure(): T` conditional never takes its true branch

**Impact: High · Complexity: Medium**

```php
/**
 * @template T of Entry
 * @param (Closure(): ?T) $callback
 * @return ($callback is Closure(): T ? T : null|T)
 */
public static function flexible(Closure $callback): ?Entry {}

function a(): Item {
    return C::flexible(function (): Item { return new Item(); }); // got Item|null
}
function b(): Item {
    return C::flexible(fn (): Item => new Item());                // got Item|null
}
function c(): Item {
    return C::flexible($this->build(...));                         // got Entry|null
}
```

A closure whose return type excludes `null` satisfies `Closure(): T`, so
the result should be `Item`. The check against the callable type fails and
the `else` branch wins. The first-class-callable form also binds `T` to its
bound (`Entry`) instead of `build()`'s return type. One large Laravel
project routes all of its cache reads through a helper with this
signature: 15 sites, plus a typed property assigned from one.

### B601. A bare generic class passed to a method template drops the class's template default

**Impact: Medium · Complexity: Low**

```php
/** @template TAsync of bool = false */
final class P {
    /** @return (TAsync is false ? Resp : Prom) */
    public function get(): Resp|Prom {}
}

/**
 * @template TAsync of bool
 * @param P<TAsync> $p
 * @return P<TAsync>
 */
function configure(P $p): P { return $p; }

takesResp($p->get());              // fine
takesResp(configure($p)->get());   // expects Resp, got Prom
```

Calling on a bare `P` applies the default `TAsync = false`. Passing the
bare `P` through a function that re-binds `TAsync` leaves it unbound, and
the conditional then picks the `else` branch outright rather than a union.
Laravel's `PendingRequest` (from `Http::acceptJson()` and friends) has
exactly this template.

### B602. A closure argument binds a template from its native return type, not its body

**Impact: Medium · Complexity: Medium**

```php
/** @return ProductCollection */
function getMostSold(): ProductCollection {
    return Cache::remember($key, $ttl, function (): Collection {
        return Product::query()->get(); // a ProductCollection
    });
    // Return type Collection is incompatible with declared return type ProductCollection
}

$banners->flatMap(fn (MidBanner $b): Collection => $b->items)
    ->filter(fn (MidBannerItem $i): bool => true);
// expects callable(Collection, Collection): bool
```

PHPStan binds `T` from the closure's inferred return (the native return
type narrowed by what the body returns). PHPantom uses the bare native
`Collection`. In the `flatMap` case the bare class also gets bound as both
the key and the value of the flattened collection rather than `mixed`.

### B603. A closure parameter typed `array` is not refined by the callable's expected type

**Impact: Low · Complexity: Medium**

```php
$sum = $items->reduce(
    function (array $carry, Item $i): array { $carry['n']->id; return $carry; },
    ['n' => new Item()],
);
// Cannot verify property 'id' — type of '$carry['n']' could not be resolved
```

`reduce()`'s callback takes `TReduceInitial|TReduceReturnType`, here
`array{n: Item}`. PHPStan intersects a closure parameter's native type with
the expected parameter type; PHPantom keeps the bare `array`.

### B604. `groupBy()`'s conditional type arguments leak into a chained callback

**Impact: Medium · Complexity: Low**

```php
/** @param Collection<int, Item> $items */
$items->groupBy('id')->map(fn ($group) => $group->firstOrFail()->id);
// $group is Collection<$preserveKeys is true ? int : int, $groupBy is array ? mixed : Item>

$g = $items->groupBy('id');            // Collection<array-key, Collection<int, Item>> (correct)
```

The call's own result type resolves the conditionals inside
`Enumerable::groupBy()`'s return, but when the call is the receiver of a
chained method, the callback parameter receives the type arguments with
the `$preserveKeys is …` / `$groupBy is …` conditionals still unresolved.

## Miscellaneous

### B586. `@not-deprecated` is lost on a subclass that inherits the method

**Impact: Low · Complexity: Low**

```php
interface V {
    /** @deprecated Gone in 4.0. */
    public function visit(): void;
}

abstract class AV implements V {
    /** @not-deprecated Only the interface declaration is deprecated. */
    public function visit(): void {}
}

class Child extends AV {
    public function go(): void { $this->visit(); } // 'Child::visit' is deprecated
}
```

`$this->visit()` inside `AV` itself is clean, so the tag is honoured where
it is written. A subclass that inherits `visit()` without overriding it
gets the interface's deprecation back, so inheritance enrichment seems to
re-propagate `deprecation_message` from the interface rather than taking
the inherited method as `AV` resolved it. PHPStan's `@not-deprecated`
stops the inheritance for every descendant.

### B588. A method that reads `func_get_args()` is not treated as variadic

**Impact: Medium · Complexity: Low**

```php
Socialite::shouldReceive('driver');   // Expected 0 arguments, got 1
$query->select('id', DB::raw('...')); // Expected at most 1 argument, got 2
$request->only('a', 'b');             // Expected at most 1 argument, got 2
$model->load('a', 'b');
```

Laravel declares these with no parameters, or one, and reads the rest
through `func_get_args()` (`Facade::shouldReceive`, `Builder::select`,
`Request::only`, `Model::load`/`with`/`isDirty`/`wasChanged`,
`ResponseTrait::cookie`, …). With `extra-arguments = true` every such call
is reported: 251 sites in one large Laravel project. PHPStan marks a
function or method whose body calls `func_get_args()` as variadic.

**Fix:** Record during parsing whether a function or method body calls
`func_get_args()` (outside nested closures and classes) and treat it as
variadic in the argument-count check and in signature help.

### B589. A docblock `object|null` subject is reported as unresolved

**Impact: Low · Complexity: Low**

```php
final class R {
    /** @return object|null */
    public function obj() { return null; }

    public function a(?object $n): void {
        $n->projects ?? [];           // clean
        $this->obj()->projects ?? []; // type of '$this->obj()' could not be resolved
    }
}
```

With `unresolved-member-access` on, a native `?object` subject is accepted
but the same type written as a docblock `object|null` reports the subject
as "could not be resolved", which is the message reserved for a gap in
PHPantom's own inference. Laravel's `Http\Client\Response::object()`
returns exactly this.

### B611. `use X as Y` resolves `X` through an earlier import

**Impact: Medium · Complexity: Low**

```php
namespace App;

use App\Facades\ArrayObject;
use ArrayObject as Real;

final class Svc2 {
    public function a(): void {
        $x = Real::STD_PROP_LIST; // Member 'STD_PROP_LIST' not found on class 'App\Facades\ArrayObject'
    }
}
```

A `use` statement's target is always fully qualified, so `Real` is the
global `\ArrayObject`. PHPantom resolves it through the file's own import
of the same short name. The real-world shape is
`use Illuminate\Support\Facades\Redis; use Redis as PhpRedis;`, where every
`PhpRedis::OPT_*` constant and `PhpRedis` parameter lands on the facade.

### B612. A `@method` parameter defaulting to `null` is not implicitly nullable

**Impact: Low · Complexity: Low**

```php
/** @method static void tag(string $disk = null) */
final class S {
    public static function native(string $disk = null): void {}
}

S::tag(null);    // Argument 1 ($disk) expects string, got null
S::native(null); // fine
```

PHP makes a parameter with a `null` default implicitly nullable, and
docblock signatures follow the same convention (PHPStan treats them so).
Generated facade tags use it widely, e.g. Laravel Excel's
`toCollection(..., string $disk = null, ...)`.

### B614. Pest bindings outside `tests/Pest.php`'s `->in()` are ignored

**Impact: Low · Complexity: Medium**

```php
// tests/Feature/GiftTest.php
uses(FeatureTestCase::class, CreatesProducts::class);

it('gives a gift', function (): void {
    $this->createProductCollection(5); // not found on PHPUnit\Framework\TestCase
});
```

`$this` in a Pest test closure is read from the `pest()->extends()` and
`uses()` chains `tests/Pest.php` binds with `->in()`. Three cases are not:

- A `uses()` or `pest()->extends()` written in the test file itself, with no
  `->in()`, which binds that file.
- Traits in the binding. Pest builds the test class from the bound test case
  plus every bound trait, but only the class is used for `$this`, so a
  helper that lives on a trait is not found.
- A test directory other than `tests/` (Pest's `--test-directory`).
