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

No outstanding items.
