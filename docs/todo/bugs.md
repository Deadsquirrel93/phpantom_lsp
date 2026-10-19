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

### B649. A template's diagnostics are filtered against the wrong ranges

**Impact: Medium · Complexity: Low**

```blade
@php($user = null)
@if (false)
    {{ $user->name }}   {{-- still reported, though this branch never runs --}}
@endif
```

After the slow collectors run, `collect_slow_diagnostics_scoped`
(`diagnostics/collect.rs`) drops every diagnostic that starts inside a
branch the forward walk found unreachable. The unreachable ranges are
converted with the free `offset_range_to_lsp_range`, which stays in the
coordinates of the PHP the template lowers to, while the collectors built
their diagnostics with `Backend::offset_range_to_lsp_range`, which maps them
back into the template. The two sets of positions are compared directly, so
in a template the filter keeps diagnostics from dead branches and can drop
live ones that happen to line up. Convert the dead ranges with the Backend
method, or compare in virtual-PHP coordinates before translating. The two
functions sharing one name in `diagnostics/mod.rs` is what makes this easy
to get wrong. Rename the free one (e.g. `checked_byte_range_to_lsp_range`)
so a call site has to choose deliberately.

## Templates

No outstanding items.

## Name resolution

No outstanding items.

## Code actions

### B648. Generated property hooks are indented with four spaces in a two-space file

**Impact: Low · Complexity: Low**

```php
class User {
  public string $name;   // "Generate get hook" indents the hook body by 4
}
```

`detect_indent_unit` (`code_actions/generate_property_hooks.rs`) returns a
tab when the property's indentation contains one and four spaces otherwise.
`code_actions::helpers::indent_unit` already detects a file's unit,
including two spaces. Use it and delete the local copy.

## Miscellaneous

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

**Blocked upstream** on
[carthage-software/mago#2589](https://github.com/carthage-software/mago/issues/2589).
Don't work on this until a mago release fixes it.

mago's `recover_incomplete` parse keeps `$this->` at the end of the file
with a `Missing` member, but has no recovery for a `::` missing its member:
it reports `UnexpectedEndOfFile(OneOf([]))` and drops the whole statement
(inside a call, `foo(Mailer::` keeps `foo()` with no arguments). So
`collect_unused_import_diagnostics` (`diagnostics/unused_imports.rs`) never
sees the class it names. This can't be worked around in
`parser/recovery.rs`. A placeholder member written over the whitespace
after `::` would be reported as an unknown member, and one appended past
the end of the text would have a span outside the content.

Once mago keeps the statement, add `Mailer::` cases (with and without a
trailing newline, and inside a call) to the recovery tests and the
unused-import tests.
