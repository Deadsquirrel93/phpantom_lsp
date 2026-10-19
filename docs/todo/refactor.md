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
feature area.    

Each item must include:

- **What to do** (concrete action, not "consider refactoring X").
- **Which files to change** (list specific paths).

---

# Outstanding items

### Keep a file's state in one record

`Backend` holds about 20 maps keyed by URI, each behind its own lock:
`symbol_maps`, `parse_errors`, `file_imports`, `file_symbol_imports`,
`resolved_names`, `file_namespaces`, `file_pest_uses`, the five `blade_*`
maps, `typed_receiver_view_spans_cache`, `morph_column_spans_cache`, and
others. They are filled in `apply_ast_index_updates_batch` and emptied by a
separate hand-kept list in `clear_file_maps`, `forget_closed_file` and
`on_did_close`, so nothing makes a new map show up on the removal path.
`blade_publish_lock` exists only because three of these maps can be
published out of step. Move the per-file data into one `FileState` struct
in a single `HashMap<String, Arc<FileState>>`, publish it as a whole, and
remove it in one call. Drop `blade_publish_lock` once the Blade fields are
part of it. The doc comment on `clear_file_maps` lists 6 of the roughly 18
maps it clears; it goes away with the change.

Files: `src/lib.rs`, `src/parser/ast_publish.rs`,
`src/backend/file_access.rs`, `src/backend/documents.rs`,
`src/backend/construction.rs`, and the readers of each map.

### Split `Backend` into sub-structs

`Backend` (`src/lib.rs`) has 87 fields. 34 are framework state (24 Laravel,
7 Blade, 2 Pest) and 14 are `Arc<AtomicBool>` client capabilities that are
written once during `initialize`. `clone_for_diagnostic_worker`
(`src/backend/construction.rs`) copies every field by hand, and every
blocking request goes through it. Group the fields the way `SymbolIndex`,
`WorkspaceEnv` and `DiagnosticState` already are:

- `ClientCaps`: a plain `Copy` struct set once during `initialize`.
- `LaravelState` and `BladeState`.
- `WorkspaceIndexState`: `workspace_indexed`, `workspace_index_lock`,
  `full_index_in_progress`, `workspace_index_status`.
- A `GatedIndex<T>` for the index plus has-any flag (plus dirty flag) trio
  that `laravel_macros`, `laravel_pivots` and `laravel_commands` each
  spell out by hand.

Then hold the shared state behind one `Arc` so a request clone is a single
refcount bump instead of about 80, and the hand-written clone goes away.
The fields that are snapshotted today rather than shared (`client_name`,
parts of `WorkspaceEnv`) must stay correct; check each one's writers. Do
this after "Keep a file's state in one record", which removes many of the
fields.

Files: `src/lib.rs`, `src/backend/construction.rs`,
`src/backend/startup.rs`, `src/workspace_env.rs`, `src/backend/laravel/`,
and the field accesses across `src/`.

### Set up every request the same way

`with_file_content` (`src/backend/requests.rs`) activates the chain cache,
the type-engine caches, the resolved-class cache and the parse cache, and
its comment calls it the one place no handler can bypass. Three places do
it differently:

- `completion/handler/mod.rs` repeats the four guards by hand.
- `code_actions/dispatch.rs` activates a different subset (no chain cache,
  no resolved-class cache).
- `lsp_dispatch.rs` (the wasm build) calls `resolve_definition` and
  `handle_rename` directly, so those requests get none of the caches and
  no panic guard.

`hover/mod.rs` also activates the type-engine caches a second time inside
an already-guarded request. Extract the guards into one `RequestScope`
value that `with_file_content` and these callers all enter.

Files: `src/backend/requests.rs`, `src/completion/handler/mod.rs`,
`src/code_actions/dispatch.rs`, `src/hover/mod.rs`, `src/lsp_dispatch.rs`.

### One file context for the type engine

`ResolutionCtx`, `CtxLoaders`, `VarResolutionCtx`
(`type_engine/resolver/context.rs`), `VarQuery`
(`type_engine/variable/resolution.rs`) and `ForwardWalkCtx`
(`forward_walk/walk_ctx.rs`) repeat the same fields: the content, the
cursor offset, the current class, `all_classes`, the two class loaders, the
source imports and the backend. They are built by hand in more than 90
places, and the conversions between them drop state without saying so:
`as_resolution_ctx` hard-codes `is_in_static_method: false`,
`preserve_static: false` and `laravel_macro_this_resolver: None`. Pull the
shared fields into one borrowed `FileCtx<'a>` that each context holds by
reference, give each its own extras on top, and add constructors so call
sites stop listing fields.

Files: `src/type_engine/resolver/context.rs`,
`src/type_engine/variable/resolution.rs`,
`src/type_engine/variable/forward_walk/walk_ctx.rs`, and their
construction sites (including `resolution_tests.rs`).

### Keep the forward walk's control-flow state in `ForwardWalkCtx`

`LOOP_DEPTH` and `EXIT_EDGES` (`forward_walk/loop_control.rs`),
`RETURN_EDGES` (`forward_walk/closures.rs`) and `UNREACHABLE_RANGES`
(`forward_walk/reachability.rs`) are thread-locals, although they describe
the walk in progress. A walk's behaviour therefore depends on whatever is
running further up the call stack, and nested walks have to save and
restore them by hand. Move them into `ForwardWalkCtx` (stacked for the
closure and loop frames that need it) and return the unreachable ranges
from the walk instead of reading them from a thread-local afterwards.

Files: `src/type_engine/variable/forward_walk/loop_control.rs`,
`closures.rs`, `reachability.rs`, `walk_ctx.rs`, `diagnostic_walk.rs`,
`src/diagnostics/collect.rs`.

### Move `is_type_compatible` into the type layer and split it

`is_type_compatible` (`src/diagnostics/type_errors/compatibility.rs`) is a
single function of about 940 lines. It is the assignability check used by
the argument, return and property diagnostics and by Blade call-site
inference, and it sits apart from `php_type::is_subtype_of` (about 395
lines) and `class_lookup::is_subtype_of_typed`. It opens with a Laravel
model-type expansion. Move it to `src/php_type/` next to `is_subtype_of`,
split it into one function per argument type kind, and take the Laravel
expansion as a step the caller runs before the check. Split
`is_subtype_of` the same way while there.

Files: `src/diagnostics/type_errors/compatibility.rs`,
`src/php_type/subtype.rs`, `src/class_lookup/subtyping.rs`, and the
callers.

### Type call arguments from the AST, not their text

`resolve_arg_text_to_type` (`type_engine/call_resolution/arg_text_types.rs`),
`types/conditional/text_args.rs` and `const_fold.rs` work out an argument's
type from its source text, re-detecting closures, first-class callables,
array literals and casts by scanning strings. `text_args: &str` is threaded
through `owner_method_call.rs`, `template_binding.rs` and `out_param.rs`.
This is a second resolution path next to `resolve_rhs_expression`, and it
pulls `type_engine` up into `completion::source::helpers` (18 calls, e.g.
`extract_closure_params_from_text`). Pass the argument expressions instead
and resolve them through `resolve_rhs_expression`, then delete the text
path and the `completion` imports it needed.

Files: `src/type_engine/call_resolution/arg_text_types.rs`,
`src/type_engine/types/conditional/text_args.rs`,
`src/type_engine/types/conditional/const_fold.rs`, and the callers that
pass `text_args`.

### Take Laravel out of the core class loader

`find_or_load_class` (`src/resolution/classes.rs`) special-cases the
configured date class and Laravel aliases, and `find_or_load_class_typed`
applies the auth, storage, facade-alias, macro and pivot patches on every
call. Some of these are gated on bare short names (`"Guard" | "Request" |
"Auth"`, `"FilesystemManager" | "Storage"`). The generic resolved-class
cache (`virtual_members/cache.rs`) also carries `is_laravel`,
`laravel_aliases` and `schema_index`. Add a class-refinement hook that the
loader runs after loading, register the Laravel patches through it, and
match on FQNs rather than short names.

Files: `src/resolution/classes.rs`, `src/virtual_members/cache.rs`,
`src/virtual_members/laravel/`, `src/backend/laravel/`.

### Make diagnostic collectors share the pass context

`collect_slow_diagnostics_scoped` builds a `FileDiagnosticContext` so the
pass reads `symbol_maps` and `file_context` once, but only about 10
collectors take it. The rest (`class_case_mismatch.rs`, `argument_count.rs`,
`laravel_string_keys.rs`, …) read the locks again themselves.
`run_slow_collectors` (`src/diagnostics/collect.rs`) is about 200 lines of
`step!` calls, unlike the declarative `COLLECTORS` table code actions use.
Give every collector the context, list the collectors in a table, and
replace the `Vec::remove` loop in the reachability filter with `retain`
(it is quadratic in the number of diagnostics).

Files: `src/diagnostics/collect.rs` and the collector modules under
`src/diagnostics/`.

### Use the existing helpers

- `find_line_start` is defined three times (`generate_constructor.rs`,
  `promote_constructor_param.rs`, `extract_function/naming.rs`). Keep one,
  in `text_position.rs` next to `line_start_byte_offset` (which takes a
  line index, not an offset), and use it from all three.
- A line's indentation is read by `code_actions::helpers::indent_of_line_at`
  and again by `line_indent` (`phpstan/fix_return_type/edits.rs`) and
  `indentation` (`insert_translation_key.rs`). The local copies return a
  `&str` slice where the shared one allocates a `String`; make the shared
  one return `&str` and delete the copies.
- `PhpType::is_int` and the other keyword predicates call
  `keyword_lowercase` (`php_type/keywords.rs`), which allocates a `String`
  per call; use the non-allocating `with_ascii_lowercase` that the same
  file already has.
- `self.symbol_maps.read()` is called directly in 55 places against 16
  uses of `Backend::symbol_map_for`, which exists to replace them. Some
  hold the guard across real work (`completion/handler/mod.rs`,
  `references/mod.rs`).
- `Backend::resolve_to_fqn` (`definition/resolve.rs`) is a pass-through
  to `names::resolve_to_fqn` "kept for API compatibility". Point its 8
  callers at the real function and delete it.

Files: as listed.

### Distinct types for source names, FQNs and URIs

Class loaders are all `dyn Fn(&str) -> Option<Arc<ClassInfo>>`, and the
only thing that tells a name as written from a resolved FQN is a leading
`\`, added with a `format!` in `load_resolved_name`. `find_or_load_class`
accepts short, qualified and `\`-prefixed names alike. There are about 220
hand-stripped leading backslashes (`trim_start_matches('\\')`,
`strip_prefix('\\')`), `find_class_by_name` and `find_class_by_fqn`
disagree on case sensitivity, and URIs are plain `&str` in more than 500
signatures. Introduce `SourceName`, `Fqn` and `Uri` newtypes and convert
one subsystem at a time, starting with the class-loader closures in
`src/class_lookup.rs` and `src/resolution/loaders.rs`. Write a short
module doc there that says which of the roughly 20 loader entry points to
use when. This is large; plan it as several PRs.

Files: `src/class_lookup.rs`, `src/resolution/`, `src/util.rs`, then
callers subsystem by subsystem.

### Rename the type-engine modules that collide

- `type_engine/types/` shares its name with `crate::types`.
- `type_engine/types/narrowing/{instanceof,assertions,guards}.rs` (reading
  facts out of a condition) and
  `forward_walk/cond_narrowing/{instanceof,assertions,type_guards}.rs`
  (applying them to the scope) have near-identical file names, and
  `variable/resolution/guards.rs` uses "guards" for re-entry guards.
- `foreach_resolution.rs` holds iterable element types, not the foreach
  logic, which is in `forward_walk/foreach.rs`.
- `resolver/array_access.rs` and `rhs_resolution/array_access.rs` both
  resolve `$x[...]`; merge them or name them after what differs.
- `subject_expr`, `subject_extraction` and `subject_resolution` each
  describe themselves as the single shared entry point. Fix their module
  docs to say how they relate.

Files: `src/type_engine/`.
