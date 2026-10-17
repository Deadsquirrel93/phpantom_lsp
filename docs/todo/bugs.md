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

### B627. Find References can hang on a constructor or a Laravel macro

**Impact: Medium-High · Complexity: Low**

Two reference searches hold a read guard across a call that takes the same
lock again:

- `collect_constructor_hierarchy` (`references/classes.rs:281-317`) holds
  `symbols.gti_index.read()` while `declares_ctor` loads each descendant
  through `find_or_load_class`. When that load parses a file (an index
  miss, or a class the descendant's docblock types link to), publishing
  the parse ends in `index_inheritance` (`symbol_index.rs:209`), which
  takes `gti_index.write()` on the same thread. Find References on a
  `__construct` hangs, and every later parse blocks behind it.
- `collect_macro_declaring_targets` (`references/member_scope.rs:606-627`)
  holds `laravel_macros.read()` while `collect_ancestors` loads classes,
  and loading a class reads `laravel_macros` again
  (`inject_laravel_macros`, `resolution/classes.rs:207`). The lock is
  task-fair, so a macro index rebuild queued between the two reads (an
  edit to a file that calls `macro(` or `mixin(`) deadlocks member Find
  References.

Read each step under its own short-lived guard
(`let children = self.symbols.gti_index.read().get(&fqn).cloned();`), and
collect the ancestors before taking the macro index.

## Type comparison

No outstanding items.

## Standard-library return types

No outstanding items.

## Reachability

No outstanding items.

## Narrowing

### B622. Whether a by-reference callback runs now is read from the current file's text only

**Impact: Low-Medium · Complexity: Medium**

`src/type_engine/variable/forward_walk/by_ref.rs` decides whether a
callback argument is invoked immediately (so its by-reference writes
reach the variables after the call) by re-walking the current file's AST
with `with_parsed_program` and text-scanning the docblock above the callee
for `@param-immediately-invoked-callable` /
`@param-later-invoked-callable`. A callee declared in another file never
has its tag seen and gets the default, and methods are matched by short
class name within the current file. The re-parse also runs per call when
no parse cache is active. The docblock parser already reads both tags
(`docblock/parser.rs`): carry the invocation mode on `ParameterInfo` and
look it up through the function loader and the resolved class.

## Array types

No outstanding items.

## Laravel

No outstanding items.

## Blade

No outstanding items.

## Templates

No outstanding items.

## Name resolution

### B625. A qualified function call through a class import is not found without a resolved name

**Impact: Low · Complexity: Medium**

```php
namespace App;
use Lib\Helpers;
Helpers\greet(   // no signature help for Lib\Helpers\greet()
```

PHP expands the first segment of a qualified function name through the
file's class imports. `Backend::resolve_function_name_at`
(`resolution/functions.rs`) only gets that right when mago's resolved
name for the call's offset is available. Without it, the
`names::function_lookup_names` fallback gets the `use function` table
and no class imports, so it builds `App\Helpers\greet`. The callers that
pass no resolved name are `resolve_function_name` (signature help through
`callable_target.rs`, `replace_deprecated.rs`) and every
`function_loader_with(None, …)` (Blade echoes, discovery, the
fix-return-type inference). Give the function loader the class `use` map
as well (or the `FileContext` it comes from) and pass it to
`function_lookup_names`.

## Miscellaneous

### B616. PHPStan quick fixes write `\n` line endings into `\r\n` files

**Impact: Low · Complexity: Low**

In a file saved with `\r\n` line endings, the text a PHPStan quick fix
inserts (an `#[Override]` line, an `@throws` or `@return` tag, a whole new
docblock) ends its lines with `\n`, and a docblock the fix rewrites comes
back with all of its lines ending in `\n`. The file is left with mixed line
endings. `sort_use_statements` and `insert_translation_key` already use the
file's own terminator (`content.contains("\r\n")`, each with its own copy
of the check); the actions in `code_actions/phpstan/` and the shared edits
in `code_actions/docblock_edit.rs` write a literal `\n`. One
`line_ending(content)` helper in `text_position.rs` would serve all of
them.

### B618. "Update return type" on a closure writes the enclosing method's type

**Impact: Medium · Complexity: Medium-High**

```php
public function run(): array {
    $f = function (): string {
        return 42;   // PHPStan: Anonymous function should return string but returns int.
    };
    return [];
}
```

The quick fix rewrites the closure to `function (): array` and adds a
`@return array{}` docblock above the statement: the type it writes is the
method's, not the closure's. The return-type quick fixes
(`code_actions/phpstan/fix_return_type/`, `remove_unused_return_type.rs`,
`add_iterable_type.rs`) find their function by scanning text (braces back
from the diagnostic line, the `)` before the `{`, the line holding
`function`), and the inference behind them (`declared_body` in
`fix_return_type/inference.rs`) then looks for a named function or method
around the body brace, so a closure resolves to the method it sits in. An
arrow function has no brace at all, and the text scan lands on the
method's. Locate the function through the parsed AST instead
(`find_enclosing_function` in `code_actions/cursor_context.rs`, with
`FunctionKinds::All`) and read the native return type, the body to infer
from and the docblock (already `paren_function_docblock`) from that one
node.

### B626. A namespace declared on the `<?php` line is reported as a PSR-4 mismatch

**Impact: Low · Complexity: Low**

```php
<?php namespace App\Models;   // Namespace `<global>` does not match PSR-4 expected `App\Models`

final class User {}
```

`namespace_decl_from_content` (`diagnostics/namespace_mismatch.rs:76-119`)
takes the first line that starts with `namespace `, so a declaration that
shares the `<?php` line is never seen and the file counts as global. "Fix
namespace" (`code_actions/fix_namespace.rs:43`) reads the same function and
offers to insert a second declaration. The diagnostic already parses the
file (`is_structural_single_classlike_file`); read the `Namespace`
statement's name span from that AST.

### B628. A deleted file's workspace diagnostics are still reported

**Impact: Low-Medium · Complexity: Low**

With workspace diagnostics on, a file the background pass diagnosed keeps
its entries in `diag.workspace_diags` (`diagnostics/workspace.rs:411-427`)
after it is deleted: the watched-file deletion path
(`indexing/reindex.rs:122-150`) purges the file's indexes but not this
store, and the pull handler goes on reporting the URI through
`tracked_uris` (`diagnostics/pull.rs:194-200`), so its problems outlive
the file until the session restarts. Clear the native and external
entries for a deleted URI; its result id stays, so the next pull tells
the client the set is empty. Found by reading the code, not yet
reproduced in an editor.
