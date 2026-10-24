# PHPantom — Type Inference

Type resolution gaps: generic resolution, conditional return types,
type narrowing, PHP version features, and stub attribute handling.
Items that are purely about *completion UX* or *stub metadata
extraction* live in [completion.md](completion.md).

Items are ordered by **impact** (descending), then **complexity** (ascending)
within the same impact tier.

| Label      | Scale                                                                                                                  |
| ---------- | ---------------------------------------------------------------------------------------------------------------------- |
| **Impact** | **Critical**, **High**, **Medium-High**, **Medium**, **Low-Medium**, **Low**                                           |
| **Complexity** | **Low** (mechanical/boilerplate, no design decisions), **Medium** (self-contained, follows an existing pattern), **Medium-High** (spans modules, some new design), **High** (shared/core subsystem, correctness or performance tradeoffs), **Very High** (cross-cutting architecture, wide blast radius) |

---

## T26. Class constants named as docblock types (`Foo::BAR`, `Foo::BAR_*`)

**Impact: Medium · Complexity: Medium**

A class constant named in a type position is not read at all: `@param
Foo::BAR $x`, `@param self::FOO|self::BAR $x` and `@param C::class|D::class
$x` all stay as unresolved text (or fall back to the native hint), where
PHPStan and Psalm give the constant's value (`'bar'`, `'bar'|'foo'`,
`'App\C'|'App\D'`). The same constants are read when they are the operand
of `key-of<>`/`value-of<>`, through `constant_operand_shape`, so the value
lookup exists; the bare reference just never reaches it. Found porting
Psalm's `ConstValuesTest` and `ReconcilerTest`; the assertions are `// SKIP`
in `tests/psalm_assertions/const_values.php` and
`tests/psalm_assertions/type_reconciliation_reconciler.php`.

Falling back to the native hint is not harmless. A method declared
`@return self::STATUS_*` over a native `int` reads as `int`, so passing its
result to a parameter typed `1|2` is reported.

The wildcard form resolves a pattern like `Foo::BAR_*` to the union of all
matching constant types on the class. PHPStan supports this syntax in
docblock type strings:

```php
class Status {
    const STATUS_ACTIVE = 1;
    const STATUS_INACTIVE = 2;
    const STATUS_PENDING = 3;
}

/** @param Status::STATUS_* $status */
function setStatus(int $status): void { ... }
// $status should resolve to 1|2|3
```

When the type engine encounters a constant pattern containing `*`,
it should:

1. Resolve the class (`Status`).
2. Enumerate all constants matching the glob pattern (`STATUS_*`).
3. Build a union of their literal types.

**References:**
- PHPStan: `ConstantWildcardType` / constant enum resolution.
- Phpactor: `GlobbedConstantUnionType`.

---

## T49. A closure passed to a constructor gets no parameter types from it

**Impact: Medium · Complexity: Medium**

```php
class Foo {
    /** @param callable(Bar): void $cb */
    public function __construct(callable $cb) {}
}

new Foo(function ($bar) {
    $bar->  // PHPantom: nothing. Passed to a method instead: Bar
});
```

A closure passed to a function's or method's typed callable parameter gets
its parameter types from it, but `infer_callable_params_for_call`
(`forward_walk/callable_inference.rs`) only takes a `Call`, so a
constructor argument gets none. The diagnostic walk passes no call for an
`Instantiation`'s arguments (`walk_closures_in_call_args` in
`diagnostic_walk.rs`), and the cursor walk (`try_enter_closure` in
`forward_walk/closures.rs`) only enters closures among a `Call`'s
arguments. Completion, hover and diagnostics inside the body all see an
untyped parameter. An anonymous class's constructor arguments
(`walk_closure_in_partial_call_args`) have the same gap.

**Plan.** Teach the inference an `Instantiation` and an anonymous class's
argument list: resolve the class the way `new` resolves it, read the
callable parameter of `__construct` at the argument's position the way
`infer_callable_params_from_static_receiver_fw` does for a static call,
and use it from both walks so diagnostics and completion agree.

---

## T34. `static::CONST` over-narrows to the declaring class's value
**Impact: Medium · Complexity: Medium-High**

Late static binding is ignored for class constants: `static::CONST`
resolves to the value declared on the *current* class, even though a
subclass may redeclare it. This is unsound in the false-positive
direction, since the narrowed literal can drive a bogus argument-type
mismatch or mark a live `match` arm dead.

```php
class Foo {
    const NO_TYPE = 1;
    /** @var string */
    const TYPE = 'foo';

    public function doFoo(): void {
        self::NO_TYPE;      // 1      (correct)
        static::NO_TYPE;    // 1      (PHPStan: mixed — a child may redeclare it)
        static::TYPE;       // 'foo'  (PHPStan: string — a child must respect @var)
    }
}
```

PHPStan's model: `self::` always yields the declared value. `static::`
(and `$this::`) yields the value only when it cannot be overridden, i.e.
the class is `final` or the constant is `final`/`@final`; otherwise it
widens to the constant's declared type, or `mixed` when the constant is
untyped.

**Where to change:** `class_expression_name` (`src/class_lookup.rs`) maps
`Expression::Static` to the current class exactly like `Expression::Self_`,
and `$this::` lands in the same place through the expression branch.
`class_constant_type` (`type_engine/call_resolution/return_types/class_constants.rs`) then
always prefers the initializer's value and never asks whether the class or
the constant is final.

**Tests to update once fixed:** the `static::`/`$this::` assertions in
upstream's `nsrt/class-constant-types.php` were dropped when
`tests/phpstan_nsrt/class-constant-types.php` was ported (only the
`self::` cases survive); port them back. The `static::`/`$this::`
assertions in `tests/phpstan_nsrt/class-constant-native-type.php` are
`// SKIP` against this item, and so is the `$hw::C` read of a typed
constant initialised to an enum case in
`tests/phpstan_data/Rules/Constants/bug-10212.php`.

---

## T29. Definite vs possible variable existence tracking

**Impact: Medium · Complexity: High**

PHPantom currently treats all assigned variables as definitely in
scope. This causes false negatives: a variable assigned only inside
one branch of an `if` (without the other branch) is treated as
always available after the `if`. The undefined-variable diagnostic
(`diagnostics/undefined_variables/`) only checks that some write
precedes the read, whichever branch it sits in.

**Fix:** Split variable tracking into two maps:

1. **`vars_in_scope`** — variables with a definite type (assigned on
   all code paths reaching this point).
2. **`vars_possibly_in_scope`** — variables that *might* exist
   (assigned in only one branch). Accessing these without a guard
   could be flagged as "possibly undefined."

Psalm's `Context` uses exactly this split. The `vars_in_scope` map
holds `Union` types for definitely-typed variables. The
`vars_possibly_in_scope` map is a boolean set tracking variables that
might exist. After an if/else where only one branch assigns `$x`,
`$x` moves from `vars_in_scope` to `vars_possibly_in_scope` (or is
removed from `vars_in_scope` and added to `vars_possibly_in_scope`).

Additionally, contextual flags like `inside_isset` and
`inside_conditional` should suppress diagnostics about undefined
variables in those positions (accessing `$x` inside `isset($x)` is
intentional).

**Design:**

1. Add a `possibly_defined: HashSet<SmolStr>` alongside the existing
   variable type map in the forward walker state.
2. When merging branches (if/else, try/catch), variables assigned in
   only one path move to `possibly_defined`.
3. Hover shows `T|undefined` or similar annotation for possibly-defined
   variables.
4. Future diagnostic (D-series) can warn on access of possibly-undefined
   variables.

Knowing that a target is definitely set also settles `??=`: on a target
that is set and never null (a non-nullable parameter, a required shape
key) the fallback can never be assigned, so `$string ??= 1` is still
`string`. Today the fallback is always added; the assertions in
`tests/phpstan_nsrt/coalesce-assign.php` are `// SKIP` against this item.

**References:**
- Psalm: `Context::$vars_in_scope` and `Context::$vars_possibly_in_scope`
  (`Psalm\Context`)

---

## T46. Template arguments are not inferred from how the object is used later
**Impact: Medium · Complexity: High**

```php
/** @template T */
class Box {
    /** @param T $item */
    public function add($item): void {}
    /** @return T */
    public function first() {}
}

$box = new Box();
$box->add(new Foo);
$box->first()->  // PHPStan 2.3: Foo; PHPantom: mixed (`Box<mixed>`)
```

A `new` whose constructor leaves a template open erases it to its default,
then its bound, then `mixed` (`default_type_arg` in
`inheritance/generics.rs`, reached from `instantiate_class` in
`call_resolution/template_binding.rs`), and members are substituted with
that immediately. PHPStan 2.3 keeps the template open and fills it from
the rest of the body. It ships behind the `unresolvedTemplateArguments`
bleeding-edge flag, so it is not yet on by default there either.

**How PHPStan does it.** The open template becomes a placeholder type
(`UnresolvedTemplateArgumentType`) that answers every type question as its
default or bound, so nothing downstream needs to know about it. While the
body is walked, facts about each placeholder accumulate:

- a value flows into it (`add(T $v)` called with `Foo`, `offsetSet` on an
  `ArrayAccess`): a lower bound;
- the object is passed to, returned as, or assigned to something declared
  `Box<int>`: a send, with that position's variance;
- the object is passed to `mixed` or an untyped slot: it escaped, and the
  placeholder falls back to default or bound.

Lower bounds are unioned (each first checked against the template's bound,
so `add('x')` on `T of object` leaves `Box<object>`). An invariant send
wins over lower bounds. Invariant sends between two placeholders unify
them, so `$a->add(1); $b->add('x'); join($a, $b);` gives both
`Box<1|'x'>`.

The body is then walked a second time with the solved arguments. That is
where the 1-3% cost comes from: statements before the first `new` replay
from a recording, and a later statement is re-walked only if it holds a
site or mentions a variable whose type changed (a syntactic check, cached
on the AST node). Everything else replays its recorded scope delta. The
second walk stops early once no variable differs and no site remains.
(`StatementsHandler::processBodyStmtNodesTwoPass`,
`Analyser/Generics/TemplateArgumentSolver.php`.)

**What this means for an LSP.** In PHPStan an earlier line's type can
depend on a later line: `$box->first()` hovered on the line before
`takesInts($box)` reads `int`. Our cursor-bounded walk stops at the cursor's
statement, so hover would read `mixed` there and `int` below, while
diagnostics (which walk the whole body) would disagree with hover. The
facts therefore have to come from a cursor-free pass over the whole body
(`ctx.with_cursor_offset(u32::MAX)`, as `by_ref.rs`, `foreach.rs` and
`while_for.rs` already do for discovery), cached per body and content
version so hover and completion don't pay for a full walk per request. The
solved arguments seed the cursor walk before any snapshot is recorded.

Inferring from code below the cursor means half-typed code can flip a type
while the user is typing. Two rules keep that stable: union lower bounds
instead of PHPStan's "first invariant send wins" (which depends on order),
and never narrow below default or bound from a lone escape.

**First slice.** `new` with no arguments that bind the template, assigned
to a plain variable; lower bounds from template-typed method parameters
and `offsetSet`; union merging; no cross-variable unification. That covers
the common "empty collection, then fill it" shape. Returns, property
writes and sends to typed parameters can follow.

**Shared work with T47.** Both need the same machinery: a cursor-free fact
pass over the body, a per-body cache, and a "does this statement mention
one of these variables" filter to limit the re-walk. `assignment_deps.rs`
already collects the variables a statement reads and writes
(`collect_rhs_variables`, `collect_assignment_target_vars`) and has a
convergence check (`scope_has_changes`); `seed_by_ref_capture_fixed_point`
in `by_ref.rs` is the closest existing probe-then-reseed pattern. Build it
once, for whichever of the two lands first.

**Where to look:** PHPStan 2.3's `src/Type/Generic/UnresolvedTemplateArgumentType.php`,
`src/Analyser/Generics/`, `NewHandler::unresolvedArgumentList()`, and the
`tests/PHPStan/Analyser/nsrt/template-argument-*.php` files. Ours:
`rhs_resolution/instantiation.rs`, `call_resolution/template_binding.rs`,
`forward_walk/mod.rs` (`resolve_in_method_body`, next to the existing
whole-body pre-passes `seed_static_locals` and
`try_generator_yield_inference`), `forward_walk/statements.rs`
(`process_expression_statement`, where receiver calls are already visited).

---

## T47. A closure assigned to a variable gets no parameter types from where it is used
**Impact: Medium · Complexity: High**

```php
$byName = function ($a, $b) {
    return $a->  // PHPStan 2.3: User; PHPantom: nothing
};
usort($users, $byName);   // list<User>
```

A closure literal passed straight to a typed callable parameter already
gets its parameter types, generics included (`try_enter_closure_expr` →
`infer_callable_params_for_call` in `forward_walk/closures.rs` and
`callable_inference.rs`). Assigned to a variable first, it gets nothing:
the assignment branch passes no inferred types, and the cursor walk stops
before the statements that use it. This is the same blind spot from the
other direction as T46, and the payoff is the same: completion and hover
on an untyped parameter inside the body. PHPStan 2.3 ships it behind the
`closureSignaturesFromUsages` bleeding-edge flag.

**What PHPStan infers from.** Direct calls (`$f(1); $f(2);` gives `1|2`),
passing the variable to a typed or generic callable parameter (`usort`,
`array_map`, `array_filter`, `$collection->map($f)`, taking the parameter
type after template resolution), aliases and ternaries (`$d = $f; $d(1);`),
closures stored in array offsets or lists and called from there, and
`return $f` / a typed property / `@var` against a declared `Closure(int):
void`. All observed argument types are unioned. If the closure escapes
(passed to `mixed`, bare `callable` or `Closure`, `call_user_func`, a
by-ref parameter, `global`, `compact`/`extract`/`get_defined_vars`, `$$`),
or the union doesn't fit the declared type, the declared type stands. A
body is only observed when nothing outside it can reach its variables
(`isClosedBody`).

The mechanism is the two-pass walk described in T46: an untyped parameter
starts as a placeholder that remembers its closure, and since it is an
ordinary type it flows through variables, array shapes and foreach without
special cases. Call sites add lower bounds, typed targets add bounds, and
the second walk re-enters the closure body with the solved types.
(`ClosureSignatureInference`, `TemplateArgumentObserver`,
`TemplateArgumentSolver::resolveClosureSignatureObservation`; tests in
`nsrt/closure-signature-from-usages*.php`.)

**Plan.**
1. When `try_enter_closure_expr` meets `$v = <closure>` (or an array
   offset or list element) with the cursor inside it and untyped
   parameters, scan the rest of the enclosing statement list for uses of
   `$v` and its aliases. Run the syntactic escape check first and give up
   on any hit.
2. For a direct call `$v(...)`, resolve the arguments at that position.
   For `$v` passed as an argument, reuse `infer_callable_params_for_call`
   on that call to get the callable parameter types, so `usort`,
   `array_map` and `->map()` come for free.
3. Union the observations, keep the declared hint unless the union fits
   inside it, and feed the result through the existing `inferred_params`
   slot of `try_enter_closure_body` / `seed_closure_params`. Run the same
   inference from `walk_closures_in_expr` so diagnostics and completion
   agree.
4. Later: give the variable's own type the inferred signature
   (`infer_closure_literal_type`), so `$f('x')` and `->map($f)` see it.

Resolving each use's arguments must go through the shared walker (a
cursor-free walk up to that offset, per T46's shared machinery), not a
separate resolver. Lower-value cases can wait: `use (&$x)` fixed points,
nested closures typed from an expected return type, generator escapes.

**Where to look:** ours: `forward_walk/closures.rs`,
`forward_walk/callable_inference.rs`, `diagnostic_walk.rs`
(`walk_closures_in_call`, `seed_closure_params`), `rhs_resolution/function_calls.rs`
(`$f()` reads only the variable's callable return type today).

---

## T41. A reading of the callee's body overrides a declared `@param-out`
**Impact: Low-Medium · Complexity: Low**

```php
/**
 * @param-out list<string> $lines
 */
function readInto(string $path, ?array &$lines = null): int {
    $lines = [];
    // … filled through a helper the walk cannot follow …
    return 0;
}

readInto($path, $lines);
$lines; // array{}, where the tag promises list<string>
```

The tag is read: `ParameterInfo::param_out_type` holds it and
`ParameterInfo::out_type()` prefers it over the declared type and the
null-default heuristic. But `effective_out_type`
(`type_engine/call_resolution/out_param.rs`) then lets a reading of the
callee's body replace any declared out type it is a subtype of, the tag
included, so a body the walk only partly follows hands the caller a type
narrower than the author promised.

**Fix:** when the parameter carries an explicit `@param-out`, return it
without reading the body. The body reading should only sharpen a type the
author did not spell out for the write.

---

## T43. `self::TypeAlias` inside `@extends`'s generic argument is not resolved
**Impact: Low · Complexity: Medium**

Found while porting Mago's `issue_870.php`. A class can declare its own
`@type`/`@phpstan-type` alias and hand it to its own parent's template
parameter via `self::`:

```php
/** @template TData as array<string, mixed> */
abstract class Car {
    /** @return TData */
    abstract function getData(): array;
}

/**
 * @type DataArray = array{'Gewicht': int}
 * @extends Car<self::DataArray>
 */
class RedCar extends Car {
    public function getData(): array { return ['Gewicht' => 1000]; }
}
```

`getData()`'s inherited `@return TData` should resolve to the
`DataArray` shape (offering `Gewicht` for array-key completion), but it
reads as the unresolved `RedCar::DataArray`, so `TData` never gets the
shape and completion sees only the untyped `array` declared return.

Bare alias names in `extends_generics`/`implements_generics` are already
substituted (`type_engine/type_ops/aliases.rs`), but the match is an exact
comparison against a `Named` type, so `self::DataArray` never matches it,
and `qualify_self_constant` (`php_type/transform.rs`) reads it as a class
constant instead.

**Fix:** when substituting a generic argument off `@extends`/`@implements`/
`@template-implements` (parsed by `extract_generics_tag` in
`docblock/templates.rs`), recognise a `self::Identifier` that names one of
the *declaring* class's own `type_aliases` and resolve it through
`resolve_type_alias_typed` with that class as `owning_class_name`, before
substituting it into the parent's template. Mago's `issue_870.php` can be
ported once this lands.

---

## T45. Doctrine `getRepository(Entity::class)` is typed as the generic repository

**Impact: Low-Medium · Complexity: Medium**

```php
$repo = $entityManager->getRepository(User::class);
$repo->findActive(); // PHPantom: EntityRepository<User>, so findActive() is unknown
```

Doctrine's stubs type `getRepository()` as `EntityRepository<T>` (or
`ObjectRepository<T>`), but the object it returns is the entity's own
repository: the class named by `#[ORM\Entity(repositoryClass: ...)]` or by
the `repositoryClass`/`repository-class` key of a YAML/XML mapping.
PHPStan's Doctrine extension answers this with a dynamic return type
extension. Completion, hover, diagnostics, and Find References on the
repository's own methods all miss it today.

The return type belongs in the type engine, as a return-type hook for
`getRepository()` with a class-string argument, so every consumer sees
the concrete repository. The Doctrine mapping index built from YAML/XML
resources already knows those pairs; the attribute form needs the
`ClassInfo` support F24 describes.

**Where to look:** `src/framework.rs`
(`framework_doctrine_repository_fqns_for_entity`) for the YAML/XML
mapping pairs, and method-call return resolution under
`src/type_engine/variable/rhs_resolution/` for where the hook goes.


---

## T50. Port the PHPStan Symfony and Doctrine extensions' type fixtures

**Impact: Medium · Complexity: Medium**

PHPStan's framework extensions, `phpstan/phpstan-symfony` and
`phpstan/phpstan-doctrine`, are the reference for what a Symfony or
Doctrine project expects its types to be. Their `tests/Type/**/data/*.php`
files are `assertType()` fixtures, the format `tests/assert_type_runner.rs`
already runs for PHPStan's and Psalm's own corpora. Porting them is the
measuring stick for T51 to T53, F42 to F44, and D30 to D31: each item there
lands when its fixtures pass, and the ones that cannot yet carry a `// SKIP`
naming the item.

Two things have to be settled first:

- **Where the framework classes come from.** The runner's backend has the
  embedded stubs only, and these fixtures call into `symfony/*` and
  `doctrine/*` classes. Either the fixtures get a project of their own with
  a committed `composer.lock`, installed in CI the way `examples/symfony/`
  is, or each fixture gets the slice of the framework it touches as stubs.
- **What stands in for a booted application.** The extensions read the
  compiled container XML (`containerXmlPath`), an object manager
  (`objectManagerLoader`), and a console application
  (`consoleApplicationLoader`). PHPantom reads the same facts statically,
  from the service declarations, the entity mappings, and each command's
  `configure()`, so a fixture that depends on the loader gets that setup
  written as source next to it rather than being dropped.

The extensions' rule tests (`tests/Rules/**`, expected messages per line)
port the same way onto the diagnostic fixtures `tests/fixture_runner.rs`
runs. Keep the upstream expectation and mark a gap `// SKIP` rather than
loosening the comparison, as for the other corpora.

**Where to look:** `tests/assert_type_runner.rs` (the `datatest_stable`
roots), `tests/fixture_runner.rs`, and the extensions' `tests/Type/` and
`tests/Rules/`.

---

## T51. Symfony return types the service container decides

**Impact: Medium · Complexity: Medium-High**

```php
$mailer = $container->get('app.mailer');      // mixed, should be App\Service\Mailer
$sender = $this->getParameter('app.sender');  // mixed, should be string
$result = $this->handle(new GetOrder($id));   // mixed, should be the handler's return type
```

A container lookup by class name is typed already. One by service ID is
not, though the service declaration says what it builds: a `class:` key or
`class` attribute, an ID spelled as the class, or a PHP configurator's
`set('app.mailer', Mailer::class)`. A parameter's type is its declared
value's. A query bus's `HandleTrait::handle()` returns what the message's
handler returns, the handler being the `#[AsMessageHandler]` class (or
`messenger.message_handler` service) whose `__invoke()` takes that message.
PHPStan's Symfony extension answers all three from the compiled container;
the static service and parameter declarations PHPantom already indexes
carry the same facts for the project's own services.

This is a return-type hook in the type engine, so completion, hover,
diagnostics, and references all see it, fed by `symfony_declaration_locations`
and its index rather than a second reading of the configuration. Services
installed bundles declare stay `mixed` until F26.

**Where to look:** `src/symfony/mod.rs`, `src/symfony/resources.rs`,
method-call return resolution under `src/type_engine/variable/rhs_resolution/`,
and PHPStan's Symfony extension (`ServiceDynamicReturnTypeExtension`,
`ParameterDynamicReturnTypeExtension`, `MessengerHandleTraitReturnTypeExtension`).

---

## T52. Symfony return types an argument decides

**Impact: Low-Medium · Complexity: Medium**

PHPStan's Symfony extension narrows a list of framework methods by their
arguments. Each is checked against T50's fixtures, since a recent Symfony
may already declare some of them with a conditional return type:

- `Request::getContent($asResource)`, `HeaderBag::get($key, $default)`,
  `InputBag::get($key, $default)` and `InputBag::all($key)`,
  `ResponseHeaderBag::getCookies($format)`;
- `Envelope::all($stampFqcn)`, `SerializerInterface::deserialize()` and
  `DenormalizerInterface::denormalize()` by their `$type`;
- `FormInterface::getErrors($deep, $flatten)`, `CacheInterface::get()` by
  the callback's return type, `KernelInterface::locateResource()`,
  `Extension::getConfiguration()`, `Command::getHelper()`, and
  BrowserKit's `getClient()`;
- `TreeBuilder::getRootNode()` and the node definitions' `end()` returning
  their parent, for the Symfony versions before 8 that do not declare them
  as generics.

Where a rule fits a conditional return type, write it as a declarative
patch on the vendor class (the way `virtual_members/laravel/patches.rs`
patches Laravel's) rather than as code. The extension's `stubs/` directory
shows which declarations it overrides wholesale.

**Where to look:** `src/virtual_members/laravel/patches.rs`, and PHPStan's
Symfony extension (`src/Type/Symfony/`, `stubs/`).

---

## T53. Doctrine collections narrowed by `isEmpty()`

**Impact: Low · Complexity: Medium**

```php
if (!$order->lines->isEmpty()) {
    $first = $order->lines->first(); // OrderLine|false, should be OrderLine
}
```

`Collection::first()` and `last()` return `T|false`; once `isEmpty()` has
been ruled out they cannot return `false`. This is a narrowing of the
receiver's state rather than of a variable's type, which the narrowing
pass does not model for method calls yet. PHPStan's Doctrine extension
also adds `matching()` to every `Collection`, which only `Selectable` ones
declare.

**Where to look:** the narrowing under `src/type_engine/`, and PHPStan's
Doctrine extension (`Type/Doctrine/Collection/IsEmptyTypeSpecifyingExtension`,
`DoctrineSelectableClassReflectionExtension`).
