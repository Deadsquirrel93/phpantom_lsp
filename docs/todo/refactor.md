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

Filed by a refactoring pass on 2026-10-13 that looked at the largest
files changed since 0.11.1 without running the full Step 3 checklist.
Each item touches its own set of files.

## 1. Split `rhs_resolution/calls.rs` by the kind of call

`src/type_engine/variable/rhs_resolution/calls.rs` is 3,071 lines, the
largest non-test file in `src/`, and resolves every call shape the RHS
resolver sees:

- Function calls: `resolve_rhs_function_call` (lines 657 to 1271) and
  the callable-return helpers after it (`joined_callable_return`,
  `callable_return_resolution`, `closure_call_return`,
  `array_callable_return`, to line 1399). Move them to
  `rhs_resolution/function_calls.rs`. `resolve_rhs_function_call` is a
  single 615-line function whose phases are already marked: pull the
  first-class callable invocation (`Foo::method(...)()`), the Laravel
  helpers (container bindings, `now()`/`today()`, `view()`, `config()`,
  the translation helpers), and the `$fn()` / `($expr)()` invocations
  into functions of their own.
- Method calls on a receiver: `MethodReceiver` through
  `resolve_owner_method_call` (lines 1400 to 2518). Move them to
  `rhs_resolution/method_calls.rs`.
- Static calls: `resolve_rhs_static_call` and `static_call_on_object`
  (lines 2519 to 2847). Move them to `rhs_resolution/static_calls.rs`.
- Laravel accessor shortcuts: `try_resolve_validated_shape` through
  `try_resolve_command_accessor_type` (line 2848 to the end). Move them
  to `rhs_resolution/laravel_accessors.rs`.

What stays in `calls.rs` is the dispatcher (`resolve_rhs_call`), the
argument raw-type helpers and closure-literal typing, about 650 lines.

## 2. Break up `resolve_target_classes_expr_inner` in `resolver/mod.rs`

`src/type_engine/resolver/mod.rs` is 2,040 lines, and this cycle changed
791 of them.

- `resolve_target_classes_expr_inner` (lines 502 to 1070) is one
  570-line `match`. Extract the `SubjectExpr::ArrayAccess` arm (lines 861
  to 1038), the `StaticAccess` arm (626 to 720) and the `This` arm (514
  to 587) into functions of their own, so the `match` only dispatches.
  The array-access function and the helpers only it uses
  (`resolved_array_access_type_to_resolved`,
  `resolve_call_raw_return_type`) can go to `resolver/array_access.rs`.
- `SpineKeys` (lines 203 to 491) builds the chain-cache keys for a
  subject's spine, a job of its own. Move it and `lookup_chain_cache`
  (185) to `resolver/chain_keys.rs`.

## 3. Share the per-member qualification in `parser/name_resolution.rs`

`src/parser/name_resolution.rs` (962 lines) qualifies the class names a
parsed declaration spells.

- `qualify_function_names` (lines 32 to 185) and the method loop of
  `resolve_parent_class_names` (532 to 650) repeat the same work for each
  parameter (`qualify_param_default`, the type hint, `closure_this_type`,
  `param_out_type`) and for the signature extras (`@throws`, template
  bounds and defaults, `@phpstan-assert` types, the conditional return).
  They differ only in which native hint stays as written: a method keeps
  its native return type, a function keeps its parameters' native hints
  (see "Source Names and Resolved Names" in `docs/ARCHITECTURE.md`).
  Extract one helper for a parameter and one for the extras, taking that
  difference as an argument.
- The resolve-then-compare idiom (`let r = ty.resolve_names(resolver);
  if r != *ty { *ty = r; }`) is spelled out about twenty times in the
  file. Add a `qualify_in_place(ty, resolver)` helper and use it.
- `resolve_parent_class_names` (251 to 746) is one 495-line loop body.
  Move the Laravel metadata block (329 to about 410: custom collection,
  factory and builder, factory model, facade accessor, policy, pivots,
  casts) into a `qualify_laravel_names` function, and the method,
  property and constant loops into functions of their own.

## 4. Fold `util::infer_type_from_literal` into the constant-value classifier

`infer_type_from_literal` in `src/util.rs` (used by
`src/code_actions/extract_constant.rs` and
`src/virtual_members/laravel/model_extraction.rs`) classifies a literal's
source text a second time, next to `infer_type_from_constant_value` in
`src/type_engine/variable/rhs_resolution/literal_text.rs`. The two
parse integers, floats, strings, booleans and `null` with separate code.
Make the `util` callers use `infer_type_from_constant_value` and widen
the literal it returns to its base type, then delete the `util` copy.
One difference to keep: the `util` version answers `array` for `[]` and
nothing for any other array literal, while the constant classifier
returns a shape for both.
