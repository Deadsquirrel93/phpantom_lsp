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

Filed by the Step 3 analysis on 2026-10-17 at `de6559136`, run over every
source file changed since 0.11.1 (542 of the 690 non-test files). This
round went for coverage rather than depth: each check ran as a scan with
a complete hit list, and every hit got a verdict, so the next analysis
only has to look at code changed after `de6559136`. The scans covered
the 170 files over 600 production lines or with logic in a `mod.rs`, the
32 functions of 300 lines or more, every clone-detector hit of 10 or more
normalised lines (exact, and with identifiers abstracted), the 92
production `unwrap`/`expect` calls and 47 discarded results, the 25 async
functions that lock and await and every guard bound across a loader
call, 143 slices with offset arithmetic, 120 parse sites, 80 deep clones,
every per-file cache, 387 linear membership checks, 319 text-scanning
lines in the type engine and request handlers, and the test helpers
shared across suites.

## 4. Keep route declaration sites in `RouteDiscovery`

Hover or go-to-definition on a route name
(`virtual_members/laravel/route_names.rs`, `resolve_route_definitions`)
re-parses every route file, include, router macro and Folio page. Keep the
declaration sites in `RouteDiscovery`. Measured on the largest sample route
table (about 530 lines of route files plus about 190 vendor lines), the
re-parse costs well under a millisecond per request, so this only pays off
on a much larger route table.

## 11. Resolve a method first-class callable from its node

`infer_closure_literal_type` (`rhs_resolution/calls.rs`) types
`$this->method(...)` and `Foo::method(...)` by handing the expression's span
text to `resolve_first_class_callable_return_type`
(`completion/source/helpers.rs`), which re-parses it as a `SubjectExpr`. Use
the `PartialApplication::Method` / `StaticMethod` node: resolve the receiver
with `resolve_rhs_expression` and read the method's return type the way the
method-call arm does (`rhs_resolution/method_calls.rs`), which currently needs
an `ArgumentList` a partial application does not have.

## 15. Make the remaining argument-text parsers quote-aware

Call arguments still travel through parts of the type engine as text
(the `ArrayFuncArgs` text impl and conditional return types are the
designed seam), and these parse that text without skipping strings:
`split_bitwise_or` (`types/call_site_returns.rs:450-473`), `split_point`
and `is_balanced` (`types/const_fold.rs`), the `find('(')`/`rfind(')')`
pair in `call_resolution/template_binding.rs:545-547` (wrong for
`new W($a)->m()`), the paren and bracket scanners in `subject_expr.rs`
(680-701, 760-825, 901, 959, 985-1053), and `resolve_literal_type`
(`call_resolution/return_types/literals.rs:169-225`), which types
`'a' === 'b'` as `string` from its first and last character. Move them onto
`text_scan` (`scan_top_level`, `find_matching_forward`,
`split_top_level_commas`).

## 16. Rebuild throws analysis on the AST and the shared resolver

`src/completion/source/throws_analysis/` (about 1,400 production lines) is
a text pipeline: `extract_function_signature` finds the next `function`
keyword after the cursor by lowercase search, `parse_param_type_map` types
`$variable->method()` receivers from the signature text alone (no
assignments, no `$this->prop->m()`), and `find_method_declaration`,
`find_method_return_type`, `extract_function_body` and
`find_catch_blocks` find structure by scanning. That is a second
type-resolution path. Its callers (`code_actions/update_docblock.rs`,
`completion/phpdoc/generation/`, `completion/phpdoc/mod.rs`,
`completion/context/catch_completion.rs`) mostly hold a complete file.
Walk the parsed function body for `throw`s and calls, resolve receivers
with the forward walker, and share the throw-point walk with
`forward_walk/throw_points.rs`. Keep a text path only for catch
completion on code that does not parse yet.

## 17. Navigate the remaining PHPStan quick fixes by AST

B618 covers the return-type fixes (`fix_return_type/`,
`remove_unused_return_type.rs`, `add_iterable_type.rs`). The other PHPStan
actions navigate by text too: `find_method_insertion_point`
(`code_actions/phpstan/mod.rs:152-256`, used by `add_override.rs` and
`add_return_type_will_change.rs`) walks lines back from the diagnostic for
modifiers and attributes, and `find_brace_match_line` (339-428, used by
`remove_unreachable.rs`) counts braces line by line. Find the method with
`cursor_context::find_enclosing_function` and read its attribute and body
spans, and delete the line scanners (`is_modifier_line`,
`is_attribute_line`, `contains_php_attribute`) with them.

## 22. Pass a context to the variable-resolution entry points

`resolve_variable_types`, `resolve_variable_types_in_program` and
`resolve_variable_php_type` (`type_engine/variable/resolution.rs:122, 203,
255`) take ten or eleven positional parameters under
`#[allow(clippy::too_many_arguments)]`, and about 20 callers spell them
out. They are a `VarResolutionCtx`'s inputs. Take the context (or a
`FileLoaders` plus name and offset) and drop the three allows.

## 26. Split the large Blade call-site inference file

- `src/blade/call_site_inference.rs` (1,294): move the refresh and
  invalidation orchestration (`refresh_blade_injected_vars` through
  `view_names_for_blade_uri`, 516-1054) into `call_site_refresh.rs`.

## 27. Split the request-handler entry points

- `src/completion/handler/mod.rs`: `handle_completion` (137-620) is 484
  lines. Split it into its stages.
- `src/hover/mod.rs`: `hover_from_symbol` (192-615) is 424 lines. Move
  each symbol kind's branch into the matching `hover/` module.
- `src/completion/phpdoc/mod.rs` (975): move the `TagDef` tables into
  `phpdoc/tags.rs` and split `build_phpdoc_completions` (523-962, 440
  lines) per docblock context.
- `src/completion/context/class_completion/mod.rs` (1,274 lines of logic in
  a `mod.rs`): move `ClassNameContext` (47-212), the affinity and sort
  ranking (300-421) and item building (442-754) into files of their own.

## 28. Split the diagnostic collectors

- `src/diagnostics/type_errors/mod.rs`: `collect_argument_type_diagnostics`
  (296-951) is 656 lines in a `mod.rs`. Move it into a named file and
  split it into phases.
- `src/diagnostics/unknown_members/mod.rs` (a directory holding only
  `mod.rs`): `collect_unknown_member_diagnostics_with_context` (176-578)
  is 403 lines. Split it into phases.
- `src/diagnostics/laravel_string_keys.rs`:
  `collect_invalid_laravel_string_key_diagnostics` (117-482) is 366 lines.
  Split the per-kind checks out of the span loop.

## 29. Split the code actions and the class rename

- `src/code_actions/update_docblock.rs` (1,289): split it into finding the
  function (226-616), deciding what changed (619-740) and rewriting the
  docblock (743-1287, with the 214-line `build_updated_docblock`).
  `build_info_for_function_like` (363-390) walks the trivia that
  `symbol_map::docblock::get_docblock_text_with_offset` already walks.
- `src/code_actions/import_class.rs` (824): move "Import all missing
  classes" (`collect_import_all_classes_action` through
  `find_all_unresolved_class_names`, 460-822) into
  `import_all_classes.rs`.
- `src/rename/class/mod.rs` (648): move the class-move half
  (`plan_class_move`, `build_class_move_edit`, `class_move_file_edits`,
  `class_move_block_edits`, `namespace_declaration_edits`, `ClassMove`,
  `FileMoveEdits`) into `rename/class/move_edits.rs`.

## 30. Split the remaining long procedural functions

- `src/inheritance/merge.rs`: `resolve_class_with_inheritance` (117-560,
  444 lines). Extract the per-parent merge inside the `extends` loop
  (177-450, with the Laravel factory binding and the template-bound
  fallback) and the interface enrichment (451-517).
- `src/type_engine/variable/forward_walk/foreach.rs`: `process_foreach`
  (309-727, 419 lines).
- `src/parser/ast_update.rs`: `build_ast_index_update` (408-792, 385
  lines).
- `src/type_engine/variable/forward_walk/cond_narrowing/instanceof.rs`:
  `commit_instanceof_narrowing` (446-800, 355 lines).
