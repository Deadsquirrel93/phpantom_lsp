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

### B646. An import used only by a `::` access at the end of the file is reported unused

**Impact: Low · Complexity: Medium**

```php
<?php
namespace App;
use App\Services\Mailer;   // reported unused until the member is typed
$mailer = Mailer::
```

The recovery reparse in `parser/recovery.rs` keeps a statement the end of
the file cuts off when it is only missing its closers or `;`. A `::` with
nothing after it is missing an operand instead, and the parser drops the
whole statement, so `collect_unused_import_diagnostics`
(`diagnostics/unused_imports.rs`) never sees the class it names.
