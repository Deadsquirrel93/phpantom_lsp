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

### B629. A method reached through a property or a chained call is not asked whether it runs its callback now

**Impact: Low · Complexity: Medium**

```php
$found = null;
$this->db->transaction(function () use (&$found) {   // tagged @param-immediately-invoked-callable
    $found = new Order();
});
$found;   // Order|null, should be Order
```

`call_invokes_arg_immediately` (`type_engine/variable/forward_walk/by_ref.rs`)
reads a method's `@param-immediately-invoked-callable` tag only when the
receiver is `$this`, a plain variable or a static class name. Any other
receiver (a property, a chained call, a parenthesised `new`) gets the
later-invoked default, so a by-reference capture keeps its pre-call type
alongside what the closure wrote. Resolving every receiver through the
shared pipeline (`resolve_rhs_with_scope`) would cover these, but
`receiver_mutation.rs` asks the question for every closure argument of every
call, so the receiver's type has to come cheaply: the type the expression
walk already resolved for it, rather than a second resolution.

## Array types

No outstanding items.

## Laravel

No outstanding items.

## Blade

No outstanding items.

## Templates

No outstanding items.

## Name resolution

### B631. Top-level code in two `namespace` blocks shares one cached subject type

**Impact: Low · Complexity: Low**

```php
<?php
namespace A {
    use function Lib\make;   // returns Lib\Widget, which has spin()
    make()->spin();
}
namespace B {
    make()->spin();          // the global make() returns Gadget: not reported
}
```

The unknown-member pass caches each subject's type under a
`SubjectCacheKey` (`diagnostics/subject_cache.rs`), and top-level code gets
the same scope, `ScopeKey::TopLevel { fn_scope_start: 0 }`, in every
`namespace` block of the file. The first block to resolve `make()` answers
for the same text in every later block, though the name means something
else there, and a class subject such as `Foo::` is shared the same way
between blocks that import different `Foo`s. Key top-level code by its
namespace block as well (the start of the `NamespaceSpan` containing the
access).

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

### B630. An unclosed call inside a braced `namespace` block drops the block

**Impact: Low · Complexity: Medium**

```php
<?php
namespace Lib {
    function greet(string $name): void {}
}
namespace App {
    use function Lib\greet;
    greet(   // until the `)` is typed, this line reads as `namespace Lib`
}
```

While a call inside a braced `namespace App { … }` block is missing its
`)`, mago's parser leaves the whole block out of `program.statements`, so
`update_ast` (`parser/ast_update.rs`) records no `NamespaceSpan` for it and
indexes none of its imports, classes or functions. An offset in it falls
into the last block the parse kept (`NamespaceSpan::containing`), and
every name typed there resolves against that block's namespace and imports
until the call is closed. Signature help runs on exactly this text, so it
shows the wrong function or none. A file that declares its blocks as
`namespace App;` keeps every block. The block survives only if the
parser's recovery ends the argument list at the block's `}` instead of
dropping the statement: upstream in `mago-syntax`, or by re-parsing with
the call closed, as signature help's text fallback already does for the
classes (`patch_content_for_signature`).
