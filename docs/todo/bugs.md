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

## Miscellaneous

### B637. An import used only by a call still missing its `)` is reported unused

**Impact: Low · Complexity: Medium**

```php
<?php
namespace App;
use Lib\Widget;   // reported unused until the `)` is typed
class Shop {
    public function make() {
        $w = new Widget(
    }
}
```

mago's argument-list recovery takes the method's `}` for an argument, so the
braces stop balancing: the method body closes on the class's `}`, the class
runs to the end of the file, and the parser drops it whole. Nothing in it
reaches the symbol map, and `collect_unused_import_diagnostics`
(`diagnostics/unused_imports.rs`) counts an import as used only when a span
there names it, so every import the class uses is reported until the call is
closed. A call at the top level of the file's last `namespace` block drops
just its own statement, with the same result. Signature help and
named-argument completion work around the dropped class by re-parsing with
`);` at the cursor (`patch_content_for_signature`), which diagnostics have
no cursor for. The parser's recovery has to end the argument list at the
`}` instead, upstream in `mago-syntax` or by a same-length reparse such as
the one `parser/namespace_recovery.rs` makes for a dropped `namespace`
block.

### B638. A monorepo's subprojects miss path repositories and are never rescanned after `composer update`

**Impact: Low-Medium · Complexity: Medium**

A workspace with no root `composer.json` goes through `init_monorepo`
(`indexing/init.rs`), which sets up each subproject with its own copy of the
Composer steps. That copy leaves out the PSR-4 mappings of path-repository
packages (`composer::extract_path_repo_psr4_mappings`), which
`init_autoload_paths` adds for a single project, so a class in a local
module that the classmap does not list yet never resolves. After startup a
`composer.json` or `composer.lock` change runs `rescan_composer_indexes`
(`indexing/scan.rs`) on the workspace root only. It reads the root's
`composer.json`, finds none, and just clears the caches, so a subproject's
vendor classes, autoload functions and constants stay as they were at
startup until a restart. The copy also never classifies the classes the
subproject's Composer classmap lists, which a single project does with
`classify_class_origin`, so where an optimized autoloader lists every
vendor class, completion ranks all of them as project code. Run the
single-project Composer setup per subproject root, and rescan the
subproject whose Composer files changed.

### B639. A `Pest.php` changed outside the editor is never re-read

**Impact: Low · Complexity: Low**

The `->in(…)` bindings a test file's closures are typed from come from the
`Pest.php` files above it, cached per directory in
`Backend::pest_configs` (`pest.rs`), including the directories found to
have none. The cache is only dropped when a file ending in `/Pest.php` is
saved in the editor (`on_did_save`, `backend/documents.rs`). One created by
`pest --init`, edited by another tool, or changed by a branch switch reaches
the server as a watched-file event, which `apply_watched_file_changes`
(`indexing/watch.rs`) re-indexes without touching that cache, so test
closures keep the old binding, or none, until a restart. Invalidate it there
as well.

### B640. Two edits count columns in bytes, so they land too far right after non-ASCII text

**Impact: Low · Complexity: Low**

```php
/** @var object{größe: int, name: string} $item */
echo $item->name;
```

An LSP column counts UTF-16 code units, but two places still turn a byte
offset into a column by hand and report the byte count. Any non-ASCII
character earlier on the line pushes the result right by its extra bytes:
go-to-definition on `name` above lands two columns into the key instead of
on its first letter. The hand-rolled conversions are object-shape key
go-to-definition (`definition/member/shape_keys.rs`, which returns the
key's byte index within its line as the `character`) and the ranges
"Remove assert" deletes (`code_actions/phpstan/remove_assert.rs`). Convert
an absolute byte offset through `offset_to_position` (`text_position.rs`)
instead.

### B641. "Generate promoted constructor" orphans each property's `@var` docblock

**Impact: Low · Complexity: Low**

```php
class Order {
    /** @var list<Item> */
    private array $items;
    public static int $count = 0;
}
```

The action deletes each property's declaration and re-declares it as a
promoted parameter, but the deleted span (`declaration_span` in
`generate_constructor.rs`, from `find_line_start` of the member) stops at
the declaration, so the `/** @var list<Item> */` above it stays. It then
documents whatever follows, here the static `$count`, whose type it now
claims is `list<Item>`. "Promote constructor parameter" deletes the docblock
with the property (`docblock_start_above_offset`,
`promote_constructor_param.rs`); this action should too.

### B642. Completion reads the cursor column as a character count, so it misfires after an emoji

**Impact: Low · Complexity: Low**

```php
$label = '🎉'; $order->
```

An LSP column counts UTF-16 code units, and a character outside the Basic
Multilingual Plane (most emoji) is two of them. Several completion paths
look the column up with `char_indices().nth(position.character)`, which
counts characters, so after such a character the cursor lands one character
too far right. The `_x;` that `patch_incomplete_member_access` splices in to
let the parser recover goes in the wrong place, and member completion after
`->` on that line comes back empty. The same lookup is in
`completion/handler/patching.rs` (both patchers), `completion/target.rs`,
`completion/array_shape.rs` (two sites) and
`completion/variable/completion.rs`. Convert through
`position_to_byte_offset` or `utf16_col_to_byte_offset`
(`text_position.rs`) instead.

### B643. A pull racing a diagnostics run can answer `Unchanged` and keep stale diagnostics

**Impact: Low · Complexity: Low**

In pull mode, a diagnostics run stores the new set in `last_full`, releases
that lock, and only then bumps the URI's `resultId`
(`diagnostics/delivery.rs`). A `textDocument/diagnostic` request landing in
that gap (`diagnostics/pull.rs`) finds the cache present and the client's
`previousResultId` still equal to the current one, and answers `Unchanged`
for a set that just changed. The editor keeps the old diagnostics until
something else triggers a refresh. Store the set and bump the id as one
step: under one guard, or bump the id before publishing the set.

### B644. "Ignore with `@phpstan-ignore`" corrupts a comment whose reason contains a comma

**Impact: Low · Complexity: Low**

```php
// @phpstan-ignore argument.type (legacy, see #12)
```

Adding a second identifier to this line, or removing one from it, splits the
list on `,` and stops it at the first ` (`, so a reason with a comma or with
nested parentheses is cut into pieces and treated as identifiers. The three
hand-rolled scans are in `code_actions/phpstan/ignore.rs`. The ignore-comment
parser in `phpstan_ignore.rs` and the stale-ignore check in
`diagnostics/stale.rs` already track parenthesis depth; the code actions
should parse through the same function (see the shared-parser item in
`docs/todo/refactor.md`).

### B645. A class in a non-UTF-8 file is indexed but never loads

**Impact: Low · Complexity: Low**

The workspace scan reads every file as bytes and decodes it with
`from_utf8_lossy` (`indexing/scan.rs`), so a class in a Latin-1 or
Windows-1252 file shows up in workspace symbols. Loading it on demand reads
the file with `read_to_string(...).ok()` (`resolution/lazy_parse.rs`), which
fails on invalid UTF-8 and is dropped without a log, so every use of the
class is reported as unknown and go-to-definition finds nothing. The same
read appears in `backend/file_access.rs`, `resolution/functions.rs` (two
sites) and `resolution/constants.rs` (two sites). Read bytes and decode
them the way the scan does, and log a read failure once per file.
