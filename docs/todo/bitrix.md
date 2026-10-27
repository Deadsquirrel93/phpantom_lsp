# PHPantom — 1C-Bitrix

Known gaps and missing features in PHPantom's 1C-Bitrix support. Today
Bitrix gets only the generic engine: there is no Bitrix-specific code.
Core classes resolve when the project's `bitrix/` directory is part of
the workspace, and nothing reads Bitrix's own declarations: the globals
the core creates, the variables a component template receives, the
class aliases its loader registers, components, or localization
messages.

The core is proprietary. It cannot be committed to this repository or
copied into a fixture project the way `scripts/extract-framework-stubs.php`
copies a slice of Symfony or Doctrine, so tests declare the handful of
core classes they need themselves, and the demo project ships without
the core. A trial distribution is free to download, and a few files of
it (`main/include.php` among them) are obfuscated, so nothing here may
depend on reading those.

Items are ordered by **impact** (descending), then **complexity** (ascending)
within the same impact tier.

| Label      | Scale                                                                                                                  |
| ---------- | ---------------------------------------------------------------------------------------------------------------------- |
| **Impact** | **Critical**, **High**, **Medium-High**, **Medium**, **Low-Medium**, **Low**                                           |
| **Complexity** | **Low** (mechanical/boilerplate, no design decisions), **Medium** (self-contained, follows an existing pattern), **Medium-High** (spans modules, some new design), **High** (shared/core subsystem, correctness or performance tradeoffs), **Very High** (cross-cutting architecture, wide blast radius) |

## What the baselines show

Two `analyze` runs ranked this document:

- **A legacy online store** (main 18.0, core committed, 2,400 files
  of its own under `local/`): 6,711 diagnostics, of which about 1,280
  are the same-named-stub bug
  ([B685](bugs.md#b685-a-same-named-class-in-a-parsed-file-hides-what-the-real-class-inherits))
  and about 480 an unused `catch` variable reported because the
  project's PHP version (7.4) was not detected. Of the 4,221 left, more
  than half sit in vendored libraries.
- **A current core** (main 26.150, Business edition trial), analysing
  the components its modules ship (3,858 files): 12,624 diagnostics.

With `unresolved-member-access` on, both show the same thing: the code
Bitrix developers write most, component templates and component
classes, is largely untyped. On the current core, 7,329 member
accesses have no type, led by `$APPLICATION` (2,279), `$USER` (886),
`$this` (812), `$DB` (173), `$CACHE_MANAGER` (65), and `$component`
(62). That is BX3 and BX4.

---

## BX1. Bitrix demo project

**Impact: High · Complexity: Low-Medium**

Every other Bitrix item is measured against this one.

- **`examples/bitrix/`**: a `local/` tree in the same shape as
  `examples/laravel/` and `examples/symfony/`, with "Try:" comments at
  each site and a README listing what to try: a module under
  `local/modules/` with a D7 entity, a component with `class.php`, a
  template, `result_modifier.php`, `.parameters.php`, and `lang/`
  files, and an event handler and an agent in
  `local/php_interface/init.php`.
- **No core in the repository.** The demo carries hand-written
  declarations of the core API it touches, signatures only, so it
  works on a fresh clone. The README explains how to unpack a trial
  distribution into a git-ignored `bitrix/` to try it against the real
  core.
- **Runtime assertions**: an `assertions.php` like
  `examples/laravel/assertions.php` needs an installed site, which a
  trial keeps for 30 days. Decide whether to ship one that runs against
  the user's own install, or check only what can be read without
  booting.

---

## BX2. Project discovery

**Impact: High · Complexity: Medium**

Recognize a Bitrix project by `bitrix/modules/main/include.php` (the
web root may be a subdirectory of the workspace, as with Drupal), and:

- **Index `bitrix/` and `local/` past `.gitignore`.** Many projects
  ignore the core, which leaves every core class unknown.
- **Skip what the core writes at runtime**: `bitrix/cache`,
  `bitrix/managed_cache`, `bitrix/stack_cache`, `bitrix/tmp`,
  `bitrix/backup`, and `upload/`.
- **Find the project's Composer project.** Bitrix projects keep it
  under `local/`, often `local/php_interface/`. The monorepo discovery
  finds it, except when the `composer.json` is git-ignored, which the
  legacy baseline project does while committing `vendor/`. Its vendored
  libraries were then analysed as the project's own code, and its PHP
  version was lost. Decide whether, inside a Bitrix root, a directory
  with `vendor/composer/installed.json` counts as a Composer project
  whatever `.gitignore` says.

**Where to look:** `detect_drupal_web_root` in `src/composer/project.rs`,
`src/classmap_scanner/discovery.rs`, and `discover_subproject_roots` in
`src/composer/mod.rs`.

---

## BX3. The globals the core creates

**Impact: High · Complexity: Medium** (depends on [T57](type-inference.md#t57-a-global-variable-is-untyped-unless-a-var-sits-above-it))

```php
global $APPLICATION, $USER;
$APPLICATION->SetTitle('Orders');   // CMain
if ($USER->IsAuthorized()) { … }    // CUser
```

The core assigns `$APPLICATION` (`CMain`), `$USER` (`CUser`), `$DB`
(`CDatabase`), and `$CACHE_MANAGER` (`CCacheManager`) to `$GLOBALS`
during bootstrap, and every page, template, and handler uses them
through `global` or directly at the top level. The assignments live in
`main/include.php`, which a trial distribution obfuscates, so reading
them is not an option: type these names from a fixed map whenever the
project is a Bitrix project.

---

## BX4. Variables a component or template file receives

**Impact: High · Complexity: Medium**

```php
// local/components/acme/order.list/templates/.default/template.php
<?php foreach ($arResult['ITEMS'] as $item): ?>
<?php $this->AddEditAction($item['ID'], $item['EDIT_LINK']); ?>
<?php endforeach; ?>
<?php $component->renderPager(); ?>
```

The core includes these files from inside a method, so they see that
method's locals:

- **`template.php`, `result_modifier.php`, `component_epilog.php`**:
  `$this` (`CBitrixComponentTemplate`), `$arResult`, `$arParams`,
  `$component`, `$templateName`, `$templateFile`, `$templateFolder`,
  `$componentPath`, `$templateData`, and the BX3 globals
  (`CBitrixComponentTemplate::__IncludePHPTemplate()`).
- **`component.php`**: `$this` (`CBitrixComponent`), `$arParams`,
  `$arResult`, `$componentPath`, `$componentName`,
  `$componentTemplate`, and the parent component's name, path, and
  template (`CBitrixComponent::__includeComponent()`).

`$component` and `component.php`'s `$this` are the class the
component's `class.php` declares when it has one. The owning component
follows from the path: `<namespace>/<name>/templates/<template>/` inside
a component directory, or `<site template>/components/<namespace>/<name>/<template>/`
inside a site template.

The standard header (`/** @var CBitrixComponentTemplate $this */`, …)
already works; most files do not carry it.

**Where to look:** how Blade seeds a view's variables
(`src/blade/shared_vars.rs`).

---

## BX5. Class aliases the loader registers

**Impact: High · Complexity: Medium** (depends on [C7](completion.md#c7-class_alias-support))

```php
class OrderTable extends Main\Entity\DataManager {}   // alias of Main\ORM\Data\DataManager
```

Since the ORM moved to `Bitrix\Main\ORM`, the old names
(`Bitrix\Main\Entity\DataManager`, `…\Entity\Query`,
`…\Entity\ReferenceField`, and about 50 more) exist only as aliases a
module registers with `Loader::registerClassAliases([...])`
(`main/include/compatibility.php`), resolved by the loader on first
use. Most entities, in the core and in projects, still extend
`Entity\DataManager`, so on a current core they lose `getList()`,
`add()`, and the rest; the baseline counts about 330 diagnostics naming
an `Entity\` class. Read the map's literal keys and values into the
alias index C7 builds for `class_alias()`.

---

## BX6. Localization messages

**Impact: High · Complexity: Medium**

```php
Loc::loadMessages(__FILE__);
echo Loc::getMessage('ACME_ORDER_LIST_TITLE');
```

```php
// lang/ru/class.php
$MESS['ACME_ORDER_LIST_TITLE'] = 'Заказы';
```

Complete, navigate, and hover message codes in `Loc::getMessage()`,
`GetMessage()`, and `Loc::getMessagePlural()`, show the text, report a
code no language file declares, and offer an "Add message" quick fix.
A file's messages live in the nearest ancestor directory that has a
`lang/` subdirectory: `<ancestor>/lang/<language>/<the file's path
relative to the ancestor>`, the default language first.

**Where to look:** the Laravel translation keys
(`src/virtual_members/laravel/trans_keys.rs`, `trans_catalog.rs`) and
`src/diagnostics/laravel_string_keys.rs`.

---

## BX7. Component names, templates, and parameters

**Impact: High · Complexity: Medium**

```php
$APPLICATION->IncludeComponent('acme:order.list', '.default', [
    'PAGE_SIZE' => 20,
]);
```

Navigate the component name to its directory (`class.php`, else
`component.php`), searching `local/components` before
`bitrix/components`, and the template name to the candidates a site
template can override (`<site template>/components/<namespace>/<name>/<template>/`,
the same under the `.default` site template, then the component's own
`templates/`). Which site template is active
is not known statically, so offer every candidate. Complete names,
templates, and the parameter keys `.parameters.php` declares.

**Where to look:** `src/virtual_members/laravel/view_names.rs`.

---

## BX8. ORM entity objects

**Impact: Medium-High · Complexity: Medium-High** (depends on BX5)

```php
$order = OrderTable::getById($id)->fetchObject();   // EO_Order
$order->getTitle();
OrderTable::query()->setSelect(['ID'])->fetchCollection();   // EO_Order_Collection
```

`DataManager::getMap()` declares an entity's fields, and the ORM
generates `EO_<Entity>`, `EO_<Entity>_Collection`, `EO_<Entity>_Query`,
and `EO_<Entity>_Result` with accessors per field at runtime. Two
sources:

- **The annotation file** `php bitrix.php orm:annotate` writes to
  `bitrix/modules/orm_annotations.php` (and to
  `local/modules/<module>/meta/orm.php` for a local module). Check
  first what indexing it already gives.
- **`getMap()` itself**, read statically, like the Eloquent model
  provider, for projects that never run the command.

Information-block entities (`Bitrix\Iblock\Elements\Element<ApiCode>Table`)
are generated from the database and stay out of scope.

**Where to look:** `src/virtual_members/laravel/model_provider.rs`.

---

## BX9. Class name case under the Bitrix loader

**Impact: Medium · Complexity: Low**

`class_case_mismatch` warns that `Entity\DateTimeField` (declared
`DatetimeField`) "loads on a case-insensitive filesystem but fails on
Linux". For a class the Bitrix loader serves, that is not so:
`Loader::autoLoad()` lowercases the name before mapping it to a file.
The legacy baseline reports 88 such warnings. The diagnostic needs to
know which loader serves the class: a module's `lib/` directory, or a
class registered with `Loader::registerAutoLoadClasses()`.

**Where to look:** `src/diagnostics/class_case_mismatch.rs`.

---

## BX10. Return types the core does not narrow

**Impact: Medium · Complexity: Medium** (see [E7](external-stubs.md#e7-stub-based-framework-patches))

```php
$shipment->getCollection()->getOrder();   // getOrder() not found on EntityCollection
```

Core methods declare base types where a concrete one is known: a
shipment's collection is a `ShipmentCollection`, and an order's basket
is a `Basket` rather than `BasketBase`. Collect the ones the baselines
hit and patch them.

---

## BX11. Module IDs, options, and events

**Impact: Medium · Complexity: Medium**

```php
Loader::includeModule('sale');
Option::get('main', 'server_name');
EventManager::getInstance()->addEventHandler('sale', 'OnSaleOrderSaved', [Handlers::class, 'onOrderSaved']);
```

- **Module IDs** in `Loader::includeModule()` and
  `CModule::IncludeModule()`: complete and navigate to the module's
  directory. A class from a module the edition lacks (the current-core
  baseline has 762, from `crm`, `socialnetwork`, and others) is used
  behind `Loader::includeModule()`; decide whether that call guards
  the class the way `class_exists()` does.
- **Option names** in `Option::get()` and `COption`: complete from the
  module's `default_option.php`.
- **Event names**: complete the ones the module raises.

**Where to look:** `src/diagnostics/existence_guards.rs`.

---

## BX12. Include paths and agent strings

**Impact: Low · Complexity: Low-Medium** (depends on BX2)

```php
require $_SERVER['DOCUMENT_ROOT'] . '/bitrix/header.php';
CAgent::AddAgent('\Acme\Agents\Cleanup::run();', 'acme.module');
```

Link an include path built from `$_SERVER['DOCUMENT_ROOT']` to the file
under the web root, and navigate and rename the class and method an
agent string calls.

**Where to look:** `src/document_links.rs`.
