# PHPantom — Refactoring

Technical debt and internal cleanup tasks. This document is the first
item in every sprint. The sprint cannot begin feature work until this
gate is clear.

> **Housekeeping:** When a task is completed, remove it from this
> document entirely. Do not strike through or mark as done.

## Sprint-opening gate process

Every sprint lists "Clear refactoring gate" as its first item,
linking here. When an agent starts a sprint, follow these steps
**in order**. No step may be skipped.

### Step 1. Resolve outstanding items

Read this document top to bottom. If there are any tasks listed in the
"Outstanding items" section at the bottom, complete every one of them.
Remove each task from this document as it is completed. After all tasks
are resolved, go to step 2.

If the "Outstanding items" section says "No outstanding items", go
directly to step 3.

### Step 2. Request a fresh session

After completing refactoring work, **stop and ask the user to start a
new session**. The analysis in step 3 must happen in a session where
no refactoring edits have been made. This prevents the analyst from
rubber-stamping work it just performed. Do not proceed to step 3 in
the same session where you completed step 1.

### Step 3. Analyze the codebase

This step produces a written analysis report. The report must be shown
to the user before any decision is made about the gate.

**Prerequisite:** You must be in a session where no refactoring edits
have been made (either a fresh session, or one where step 1 had no
work to do).

Run through **every section** of the analysis checklist below. For
each section, **actually read the relevant source files** using tools.
Do not rely on memory, summaries, or prior context. Open the files,
look at the code, and report what you find.

**Required output format.** For each checklist section, write:

1. **Which files you read** (list them by path).
2. **What you found** (specific observations with line numbers).
3. **Verdict: PASS or FAIL** with justification.

A section FAILs if it identifies work that should be done before the
sprint's feature tasks begin. A section PASSes only if you can point
to specific evidence (file sizes, grep results, code you read) that
confirms there is no problem.

"I didn't find anything" is not a PASS. "I read X, Y, and Z, checked
for A and B, and found no instances because [concrete reason]" is a
PASS.

After completing the full checklist:

- If **any section FAILed**: add concrete, actionable tasks to the
  "Outstanding items" section of this document. Each task must name
  the file(s) to change and describe what to do. Then go to step 1.
- If **all sections PASSed**: go to step 4.

### Step 4. Declare the gate clear

Remove the "Clear refactoring gate" row from the current sprint's
table in `docs/todo.md`. The sprint is now open for feature work.

This step may only be reached after step 3 produces an all-PASS
report. There is no shortcut.

---

## Analysis checklist

The checklist is scoped to the **current sprint's tasks**. Before
starting, read the sprint table in `docs/todo.md` and the linked
domain documents to understand which modules will be touched.

### 1. File size and module boundaries

- Identify the source files most likely to be touched by this
  sprint's tasks. Read each one. Report its line count.
- Any file over ~600 lines is a candidate for splitting. Look for
  natural seams: logically distinct groups of functions, multiple
  unrelated `impl` blocks, or a section that is already commented
  as a separate concern.
- Check whether any module is doing two jobs (e.g. parsing _and_
  resolution, or building _and_ formatting). If the sprint will add
  a third job to the same file, that file must be split now.
- Look for `mod.rs` files that have grown beyond a thin re-export
  layer. Logic that lives in `mod.rs` is harder to find and test.

**FAIL criteria:** A file that will be heavily modified during the
sprint exceeds 600 lines, or a module mixes unrelated concerns that
the sprint will make worse.

### 2. Test placement

- Check whether any `#[cfg(test)]` blocks exist inside `src/` files
  for the modules this sprint will touch. Inline tests are fine for
  pure unit tests on private helpers, but integration tests and
  anything that touches the `Backend` or multi-file resolution should
  live in `tests/`.
- Check whether the existing `tests/` files cover the modules the
  sprint will modify. List what coverage exists and what is missing.
- Look for test helper code duplicated across multiple test files.
  If the same fixture setup or assertion pattern appears more than
  twice, it belongs in `tests/common/mod.rs`.

**FAIL criteria:** Integration-level tests live in `src/`, or the
sprint will modify modules that have no test coverage at all, or the
same test helper is copy-pasted in three or more files.

### 3. Code duplication

- Grep for structurally similar functions across the modules the
  sprint will touch. Report what you searched for and what you found.
- Pay particular attention to: type string manipulation, AST node
  offset extraction, docblock text extraction, and `WorkspaceEdit`
  construction. These patterns tend to proliferate.
- If two code action handlers share a non-trivial pattern (e.g. "find
  the token at the cursor, determine its span, build an edit"), check
  whether a shared helper already exists or should be created before
  the sprint adds a third copy.

**FAIL criteria:** Two or more places implement the same non-trivial
logic (>10 lines of structurally similar code), and the sprint will
add another copy or modify one of the existing copies.

### 4. Performance and memory

- Look for any place where the full file AST is re-parsed inside a
  hot path (completion, hover, diagnostics) in the modules the sprint
  will touch. Re-parsing should happen at most once per request.
- Look for unbounded clones of `ClassInfo`, `MethodInfo`, or other
  large structs inside loops. These should be references or
  `Arc`-wrapped.
- Check whether any new data structures added in the previous sprint
  are stored per-file but never evicted. Unbounded growth in
  `DashMap` entries is a memory leak.
- Look for `Vec::contains` or `Vec::iter().find()` used as a set
  membership check on collections that could grow with the number of
  files. These should be `HashSet` or `DashSet`.

**FAIL criteria:** A hot path re-parses when it does not need to,
large structs are cloned in a loop, or a per-file data structure has
no eviction path.

### 5. Fragility and error handling

- Look for `unwrap()` and `expect()` calls in request-handling code
  paths (anything reachable from `server.rs`) in the modules the
  sprint will touch. A panic in a request handler crashes the language
  server. These should be `?` or explicit early returns.
- Check whether the sprint's target modules propagate errors up or
  silently swallow them with `let _ = ...` or empty `Err(_) => {}`
  arms. Silent failures produce confusing user-visible behaviour.
- Look for code that assumes a particular UTF-8 byte offset is a
  valid char boundary without checking. This is a common source of
  panics when files contain multibyte characters.
- Check whether any `Arc<RwLock<...>>` or `Arc<Mutex<...>>` is held
  across an `await` point or across a call that re-acquires the same
  lock. These cause deadlocks or unnecessary blocking.

**FAIL criteria:** `unwrap()`/`expect()` in a request handler, errors
silently swallowed in code the sprint will build on, or a lock held
across an await point.

### 6. Sprint-specific concerns

Read each feature task in the sprint and ask these questions. Answer
each one explicitly in the report:

- Will any task require touching a module that is already large or
  doing too many things? If so, it must be split now.
- Will any task duplicate logic that already exists elsewhere? If so,
  the shared helper must be extracted first.
- Will any task add a new data structure that needs an eviction path?
  The eviction must be planned before writing the feature.
- Will any task generate `WorkspaceEdit` responses? Check that the
  existing edit-building helpers (if any) are adequate, or that a new
  shared helper should be written before the first action is
  implemented.

**FAIL criteria:** Any "yes" answer to the above questions where the
prerequisite work has not already been done.

---

## What belongs here

Only add items that would actively hinder the upcoming sprint's work
or that have accumulated enough friction to justify a focused cleanup
pass. Small fixes that can be done inline during feature work should
just be done inline. Items do not need to be scoped to the sprint's
feature area, but they should be completable in reasonable time (not
multi-week rewrites that would stall the sprint indefinitely).

Each item must include:

- **What to do** (concrete action, not "consider refactoring X").
- **Which files to change** (list specific paths).

---

# Outstanding items

Filed by the Step 3 analysis on 2026-10-15, run over every source file
changed since 0.11.1. Items 1-2 are prerequisites for Sprint 8 tasks
(D26, BL1); the rest came out of the change-set sweep. Each item touches
its own set of files.

## 1. Share the constructor-initialisation facts with D26

`class_init_facts` and `ClassInitFacts`
(`src/diagnostics/readonly_writes.rs:132-150, 271-330`) work out which
properties a class's constructor is guaranteed to have initialised
(promoted parameters plus unconditional top-level `$this->x = …`). D26's
declaration-side check needs exactly that set. Move both, with `is_this`,
into a module of their own under `src/diagnostics/` as `pub(crate)` so
D26 builds on them instead of writing a second scan.

## 2. Split the use-line deletion edits out of `remove_unused_import.rs`

`src/code_actions/remove_unused_import.rs` has 688 production lines doing
two jobs: the code action handlers (37-193) and the edit builders that
delete a `use` line, a group member or a Blade `@use` directive (194-688),
which `fix.rs` also calls through the re-export in `code_actions/mod.rs`.
BL1 changes both halves. Move 194-688 (`range_touches_use_import`,
`build_line_deletion_edit`, `build_use_directive_deletion_edit` and their
private helpers) with their unit tests into
`src/code_actions/use_line_deletion.rs`.

## 3. Remove per-edit and per-call waste in indexing and resolution

- `symbols.method_store` (`src/symbol_index.rs:225-249`) is filled on
  every parse and evicted on every edit with a full-map `retain` per
  FQN, but nothing outside tests and `mem_audit.rs` reads it. Delete
  `MethodStore`, its population and eviction, the tests that assert on it
  (`indexing/watch.rs`, `resolution/tests.rs`, `parser/ast_update.rs`),
  its `mem_audit.rs` probe, and the "first step toward" comment in
  `src/types/method.rs:13-19`.
- `src/parser/ast_update.rs:1525-1531` scans every class in
  `uri_classes_index` on an edit that evicts classes, allocating
  `class.fqn().to_string()` per class for a linear `contains`. Binary
  search the already sorted `evicted_fqns` by `&str`, or look the evicted
  FQNs up through their URIs.
- `rhs_resolution/static_calls.rs:93-98` deep-clones the owner class
  (`Arc::unwrap_or_clone` on a cached `Arc`) on every `Foo::bar()`
  resolution though `static_call_on_class` takes `&ClassInfo`; the same
  happens in `closure_resolution.rs` (`find_owner_by_name`, ~590-600, 715,
  719) and `forward_walk/callable_inference.rs:400`. Keep the `Arc`
  (`ResolvedType::from_arc`).

## 4. Split the large type-engine files along their seams

- `src/type_engine/types/conditional.rs` (1,935 production lines): move
  the 230-line `class-string` branch (309-540) into its own function and
  the argument-form classification (742-1090) into
  `conditional/arg_form.rs`.
- `src/type_engine/variable/forward_walk/assignment.rs` (1,960): move the
  assert, guard and `@phpstan-self-out` narrowing (1393-1960) to
  `forward_walk/assert_narrowing.rs`.
- `src/parser/ast_update.rs`: `apply_ast_index_updates_batch` (988-1672)
  is one 684-line function. Extract its function/define publication
  (1189-1376), resolved-class cache invalidation (1417-1572) and Laravel
  candidate refresh (1574-1640) phases.
- `src/type_engine/call_resolution/template_subs.rs`: move constant and
  type-operator operand evaluation (1138-1391) to
  `call_resolution/constant_operands.rs`.
- `src/php_type/normalize.rs`: move the runtime-value join and
  containment (265-333, 407-681) to `php_type/runtime_join.rs`.

## 5. Move logic out of four `mod.rs` files

- `src/type_engine/variable/rhs_resolution/mod.rs`: the branch and arm
  helpers (274-1083) to `rhs_resolution/branches.rs`.
- `src/inheritance/mod.rs`: `resolve_class_with_inheritance` (173-621)
  and the rebind code (621-779) to files of their own.
- `src/virtual_members/laravel/mod.rs` (~1,100 lines of logic after the
  `mod` declarations): collection rewriting (379-640) to
  `collections.rs`, `expand_model_type` and its helpers (641-890) to
  `model_type.rs`, table-name derivation and pluralisation (936-1018) to
  `table_names.rs`.
- `src/hover/mod.rs`: the Laravel and Blade hover (626-1002:
  `hover_laravel_string_key`, `blade_block_detail`,
  `gate_ability_detail`, `try_model_property_hover`) to `hover/laravel.rs`
  next to `laravel_trans.rs`.

## 6. Split `virtual_members/laravel/provider_resources.rs`

2,779 lines (~1,640 production) built around one ~400-line closure in
`extract_provider_resources` (421-819), doing four jobs: the data model
and `ProviderScans` (1-420), container bindings (583-667, 821-910,
1356-1540), Blade and view registration (930-1170), and spatie
package-tools plus path resolution (1195-1356, 1543-1640). Make it a
directory with `mod.rs` (the model), `extract.rs`, `bindings.rs`,
`views.rs`, `package_tools.rs` and `paths.rs`. Build `PathContext` once
instead of three times inside the closure (484, 514, 677) and have
`resolve_path_arg` take it rather than the same five arguments at six
call sites.

## 7. Test placement

Seven tests in `src/parser/ast_update.rs` (2035-2110, 2201, 2247) use only
`Backend::new_test` and `update_ast`; move them to `tests/integration/`.

## 8. Small leftovers

- `src/php_type/normalize.rs:766-777`: the doc comment for
  `has_duplicate_members` is merged into `absorb_exact_class_names`'s.
- `resolved_type_with_lookup`
  (`src/type_engine/variable/rhs_resolution/mod.rs:508`) never uses its
  `_current_class_name` parameter.
