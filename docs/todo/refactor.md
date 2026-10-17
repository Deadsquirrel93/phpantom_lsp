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
shared across suites. Item 1 is a prerequisite for E1. B626 and B628
went to `bugs.md`.

## 1. Split `definition/member/mod.rs` before E1

E1 changes member go-to-definition, which lives in a 989-line `mod.rs`
whose `resolve_member_definition_with` (128-467) is one 340-line
function. Move the Laravel forwarding lookups interleaved with the
generic ones (`eloquent_array_entry_location` 498,
`find_facade_forwarded_method` 623, `macro_definition_location` 650,
`scope_method_name` 811, `find_builder_forwarded_method` 837,
`find_scope_on_builder_model` 877, `timestamp_property_to_constant` 965)
into `definition/member/laravel.rs`, split `resolve_member_definition_with`
into its lookup phases, and fold its two copies of the Eloquent
array-entry fallback (277-295 before `classify_member`, 363-380 after)
into one helper.

## 2. Release index guards before loading or long work

These hold a read guard across work that loads classes, parses or walks
the disk. No cycle was found for them yet, but the next caller that adds
one deadlocks, and meanwhile each stalls the writers queued behind it:
`resolution/classes.rs:383` (an `if let` temporary of
`workspace_root.read()` across `parse_and_cache_file`),
`resolution/functions.rs:153` (the `stub_function_index` read held to 247
across `global_functions.write()`, the reverse of the order
`parser/ast_publish.rs:591, 651` takes them in),
`completion/handler/blade_component.rs:295` (held to 329 across class and
type resolution), `semantic_tokens.rs:150` (the `blade_virtual_content`
read held through `collect_tokens`; use `blade_virtual_php_arc`),
`highlight/mod.rs:41` (the `symbol_maps` read across `file_context` and
class loading), `definition/resolve.rs:518` (`psr4_mappings` across
`resolve_class_in_file`, which parses) and `indexing/watch.rs:70-72`
(three read guards across the change loop, directory walk included).
Copy out what each needs and drop the guard first.
`definition/resolve.rs:629-641` is safe only because edition 2024 drops
`if let` temporaries before the `else`; bind them to `let`s.

## 3. Panics in request paths

- `symbol_map/extraction/laravel/gates.rs:68` slices
  `name[name.len() - 4..]`, which panics when the fourth byte from the end
  is inside a multibyte character (`$€ab->can('x')`; PHP allows any byte
  above 0x7f in a name). `update_ast`'s `catch_unwind` turns that into a
  "Parse failed (internal error)" diagnostic and a file whose index stops
  updating.
- `completion/laravel_string_keys/context.rs:315` slices
  `clause[open + 1..close]` with `close` from `rfind('}')`, which can come
  before `open` while a `use` statement is half typed;
  `code_actions/use_line_deletion.rs:391` has the same shape. Use `get(..)`.
- `string_key_candidates` (`completion/laravel_string_keys/mod.rs:113`)
  `expect`s a config sub-prefix that only the context detector's
  construction guarantees. Carry the prefix inside the config-resource
  context so the type says it is there.

## 4. Close the parse-cache gaps

These parse the same text again for each lookup instead of once per
request:

- The slow diagnostic pass returns for a Blade template
  (`diagnostics/incremental/mod.rs:260-268`) before it installs the parse
  cache (281), so every collector, and every nested lookup in the
  template's forward walk (`property_access.rs:646`,
  `readonly_properties.rs:86`, `closure_resolution.rs:119`,
  `variable/resolution.rs:178, 327`, `forward_walk/by_ref.rs:225, 276`),
  parses the virtual PHP again on every keystroke. Install
  `with_parse_cache_arc` over the virtual PHP first.
- `call_resolution/out_param.rs:340` parses the callee's file outside the
  cache, so the walk over its body re-parses it at each nested lookup.
  Register it the way `target_cache.rs:330` does, and drop the comment at
  338, which no longer holds.
- Blade call-site inference parses each caller with no cache
  (`blade/call_site_inference.rs:1079, 1257`, `blade/typed_receiver.rs:119`)
  and then resolves each passed variable through lookups that parse the
  caller again. Install the cache at the top of `extract_call_site_vars`,
  `extract_component_call_site_vars` and `typed_receiver.rs:104`.
- `virtual_members/laravel/validation_rules.rs:425, 607` call
  `parse_file_content` on the request's own file at every
  `->validated()`/`safe()` call site and request-field completion; use
  `with_parsed_program`. `:920` re-parses the FormRequest, its parents
  (738) and traits (799) per call site; memoise its rules per FQN.
- `virtual_members/laravel/macros.rs:127` parses the request file directly
  for `$this` in a macro; the first lookup into a stub file parses it three
  times (`resolution/functions.rs:157, 199, 202`); `facade_macro_concrete`
  (`laravel/aliases.rs:288-298`) reads and parses the facade's source for
  every `$this` in a `Facade::macro()` closure. Memoise the accessor in
  `LaravelAliases`.
- Hover or go-to-definition on a route name
  (`virtual_members/laravel/route_names.rs:728`) re-parses every route
  file, include, router macro and Folio page. Keep the declaration sites
  in `RouteDiscovery` (measure on a large route table first).

## 5. Keep the `Arc` instead of deep-copying classes

- `ResolvedType::apply_narrowing` (`types/resolved_type.rs:459-466`)
  copies every class a variable holds into a `Vec<ClassInfo>` for each
  `instanceof`, assertion or type-guard condition the walker meets, and
  the helpers in `narrowing/instanceof.rs` (59, 75-82, 171) and
  `narrowing/resolve.rs:70` take owned classes too. Work on
  `Vec<Arc<ClassInfo>>`.
- `type_engine/types/resolution.rs:549` `Arc::unwrap_or_clone`s the cached
  generic specialisation, which is always shared, on every resolution of a
  `Builder<User>`-style type, then re-runs the template-mixin and
  builder-scope post-processing; the fresh `Arc` also defeats the
  `Arc::ptr_eq` fast path in `scope_state/merge.rs:502`. Cache the
  post-processed specialisation under its (FQN, arguments) key.
- `call_resolution/facade_owner.rs:198, 217, 227` copy the facade's
  concrete class on every facade static call though both callers only
  borrow it. The same goes for `arg_type_resolution.rs:344`
  (`map(Arc::unwrap_or_clone)`), `raw_type_inference.rs:885`
  (`(*cls).clone()`; `ResolvedType::from_both_arc` exists),
  `resolver/mod.rs:565` and `closure_resolution.rs:107` (the current class
  per `self::`/`static::`), `narrowing/assertions.rs:291` (a `MethodInfo`
  per assertion call) and `parser/ast_publish.rs:276` (the previous classes
  of every published file).
- `get_classes_for_uri` (`backend/file_access.rs:145`), documented as a
  test helper, copies every class in a file and has six production callers
  (`rename/prepare.rs:508, 514`, `rename/class/layout.rs:225`,
  `references/classes.rs:244`, `references/dispatch.rs:203`,
  `definition/resolve.rs:179`). Move them to `shared_classes_for_uri`.

## 6. Evict per-file and per-project state

- `diag.workspace_diags` (`diagnostics/workspace.rs:411-427`) has no
  per-URI eviction, and the watched-file deletion path
  (`indexing/reindex.rs:122-150`) does not touch it: B628.
- `fqn_origin_index`, `autoload_function_origin_index` and
  `autoload_constant_origin_index` only grow (`indexing/scan.rs:116-162,
  697-720`, `indexing/init.rs:251`). A vendor rescan prunes `fqn_uri_index`
  but not these, and neither does the deletion purge.
- `clear_file_maps` (`backend/file_access.rs:464-506`), the `did_close`
  path for a file outside the workspace index, keeps that URI's
  `parsed_uris`, `uri_globals_index`, global function and define entries
  and `gti_*` edges for the rest of the session.
- `MIXIN_CACHE` (`virtual_members/phpdoc.rs:38-61`) is a thread-local
  keyed by FQN behind a process-global generation counter, so one
  `Backend`'s classes can answer for another's (its doc comment asks every
  test backend to clear it). Keep it on the resolved-class cache instead.

## 7. Remove the unused-import text safety net

`diagnostics/unused_imports.rs` (219-235, 298-429) keeps an import alive
when its alias appears anywhere in the file's text outside `use` and
declaration lines, a string or comment included. That suppresses a
diagnostic by text, which the conventions forbid. Gate it behind an
environment variable, run the test suite and the sample projects to find
what it was covering, fix those references in the symbol map, then delete
it.

## 8. Read statement `@var` docblocks from trivia in the forward walk

The walker finds the `/** @var */` block before a statement by scanning
`content[..offset]` backwards: `preceding_docblock_text`,
`trailing_docblock` and the rest of `forward_walk/var_docblocks.rs`
(14-423), a weaker copy before a `foreach` (`foreach.rs:443-466`:
`ends_with("*/")` plus `rfind("/**")`, the plain-comment bug `7f779798b`
fixed in two other copies), and `adjacent_docblock` /
`docblock_ending_at` (`forward_walk/closure_params.rs`), which find a
closure's own docblock. `trailing_docblock` also loses the
docblock when a `/*` sits inside a string between it and the previous
comment (`glob('src/*.php')`). Both walk entry points hold
`program.trivia` (`variable/resolution.rs:178, 203`,
`diagnostic_walk.rs:183`); pass it in through `ForwardWalkCtx` and look
the comment up from the statement node the way
`docblock::get_docblock_text_for_node` does.

## 9. Read declaration docblocks from the declaration node

`extract_preceding_docblock` and `declaration_preamble_start`
(`variable/resolution/template_bounds.rs:276-316`) find a function's
`@template`, `@param` and `@return` again by scanning backwards from its
start offset, for every body the walker enters (`walk_ctx.rs:234-249`
`for_declaration`, `param_seeding.rs`, which closure parameters go through
too, and `resolution/cursor_declarations.rs:127, 158`).
`node_param_has_invocation_tag` (`forward_walk/by_ref.rs:315-336`) does the
same inside a `with_parsed_program` that already holds the node. Read
these from the node's docblock (item 8's trivia), or from the parsed
`template_param_bounds` that methods and functions already carry.

## 10. Replace the `docblock/source_scan.rs` lookups

`find_var_raw_type_in_source`, `find_iterable_raw_type_in_source` and
`find_inline_var_docblock` walk backwards line by line with per-line brace
and quote state, so a multi-line string or heredoc throws them off. The
engine calls them at 16 sites in 14 files (`forward_walk/foreach.rs` ×3,
`cond_narrowing/in_array.rs`, `param_seeding.rs`, `template_bounds.rs`,
`resolution/arguments.rs`, `raw_type_inference.rs`, three in
`rhs_resolution/`, two in `resolver/`, two in `call_resolution/`, and
`completion/array_shape.rs`), and `forward_walk/closure_params.rs` reads a
closure's `@param` through `find_iterable_raw_type_in_source` too. Most of them run inside the forward walk,
whose scope already applies the `@var`. Move each caller to the walked
scope (or item 8's trivia lookup) and delete the module when the last one
goes.

## 11. Pass expressions, not their text, inside the type engine

- `resolve_receiver_types` (`variable/closure_resolution.rs:494-510`) turns
  a receiver `Expression` its callers hold back into source text and
  resolves it through `resolve_target_classes`. So do
  `resolve_foreach_expr_via_subject` (`forward_walk/foreach.rs:1259-1279`),
  `forward_walk/callable_inference.rs:143` (`Some("$this")` by text) and
  the base-variable lookup in `foreach.rs:357-391` (`find("->")` on the
  iterable's text). Resolve the expression with `resolve_rhs_with_scope`.
- `infer_closure_literal_type` (`rhs_resolution/calls.rs:468-613`) detects
  a generator through `infer_generator_type_from_closure_yields`
  (`completion/source/helpers.rs:231-301`), which finds the body with
  `find('{')` and so never sees `fn () => yield $x`, and resolves a method
  first-class callable from its span text (576-586). Use
  `parser::for_each_own_yield` with `resolve_rhs_expression`, and the
  `PartialApplication` node.
- `is_cursor_in_self_assignment_rhs` (`resolution/cursor_declarations.rs:
  18-52`) searches the cursor's line for the text `"$x = "`, missing
  `$x=$x`, `.=` and a line break after `=`; its caller already parses the
  file (`variable/resolution.rs:327`).
- `find_anonymous_class_containing_cursor` (`variable/resolution.rs:
  500-715`) is a 216-line hand-written expression walk;
  `cursor_context::innermost_at` answers the same question.

## 12. Compute diagnostic exemptions from the AST

- `diagnostics/existence_guards.rs` (about 530 lines) finds `isset()` and
  `empty()` arguments and `function_exists`/`class_exists`/`method_exists`
  guards byte by byte, matching inside strings and comments and taking any
  `if` within 200 bytes as the guard. `compute_attribute_ranges`
  (`unknown_classes.rs:267-298`) opens a range at any `#[`, one inside a
  string included. The module's reason ("the buffer often does not
  parse") does not hold: its callers (`unknown_functions.rs:93`,
  `unknown_classes.rs:89`, `unknown_members/mod.rs:194, 199`) only judge
  symbol-map spans, which come from the AST. Record the ranges in one AST
  walk (the symbol map already tags `ClassRefContext::Attribute`).
- `collect_var_annotations` (`undefined_variables/offset_guards.rs:35-67`)
  takes the first `$` after `@var`, so `@var Closure(int $x): void $cb`
  defines `$x`. Use `parse_var_docblock_pairs` on the comment trivia.

## 13. Record definition positions at parse time

- A virtual member's go-to-definition falls back to scanning every line of
  the file for an `@property … $name` or `@method … name(` tag
  (`definition/member/file_lookup.rs:188-276`, and
  `definition/implementation.rs:1005-1031`), taking the first match in the
  file rather than the one in the declaring class's docblock. Record the
  tag's offset as the virtual member's `name_offset` when the docblock is
  parsed.
- `find_eloquent_array_entry` (`definition/member/shape_keys.rs:166-230`)
  scans lines for `$casts`, `$fillable` and the rest by substring (a
  `$this->attributes[…]` read opens a "target array" too), matches values
  as well as keys, and returns a byte column. `model_extraction.rs`
  already walks these arrays; record each key's offset there.

## 14. Smaller text lookups with an AST answer at hand

- `namespace_decl_from_content` (`diagnostics/namespace_mismatch.rs:
  76-119`, also used by `code_actions/fix_namespace.rs:43`) takes the
  first line that starts with `namespace `: B626. The same request
  already parses the file.
- Hover decides a class reference is `new Foo` by text
  (`hover/mod.rs:435-440`); the symbol carries `ClassRefContext::New`.
- `try_model_property_hover` (`hover/laravel.rs:293-351`) reruns
  completion's partial-code detectors on complete code; the symbol map's
  `CallSite` already names the call and the argument.
- `find_documented_class_at_offset` (`class_lookup.rs:63-97`) pairs a
  docblock with its class by text; store the class docblock span on
  `ClassInfo` when the parser reads it (`parser/classes.rs:182`). The enum
  backing type (`diagnostics/enum_errors.rs:95-125`) and the class name
  position (`diagnostics/class_name_mismatch.rs:54-71`) are read from text
  the same way.

## 15. Make the remaining argument-text parsers quote-aware

Call arguments still travel through parts of the type engine as text
(the `ArrayFuncArgs` text impl and conditional return types are the
designed seam), and these parse that text without skipping strings:
`split_bitwise_or` (`types/call_site_returns.rs:450-473`), `split_point`
and `is_balanced` (`types/const_fold.rs`), the `find('(')`/`rfind(')')`
pair in `call_resolution/template_binding.rs:545-547` (wrong for
`new W($a)->m()`), the paren and bracket scanners in `subject_expr.rs`
(680-701, 760-825, 901, 959, 985-1053), and `resolve_literal_type`
(`call_resolution/return_types.rs:1420-1476`), which types `'a' === 'b'`
as `string` from its first and last character. Move them onto
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

## 23. Split the parser's class extraction

- `src/parser/classes.rs` (1,717 production lines) is two functions:
  `extract_classes_from_statements` (300-909, 610 lines) and
  `extract_class_like_members` (920-1683, 764 lines). Make it a directory
  and give each member kind (methods, properties, constants, enum cases,
  trait uses and adaptations) and each class-like kind its own function.
- `src/parser/mod.rs` (725 lines): move parameter and property extraction
  (`extract_hint_type` through `extract_property_info`, 89-420) into
  `parser/params.rs`, leaving the `parse_php*` entry points.

## 24. Split the call-resolution and conditional-type files

- `src/type_engine/call_resolution/template_subs.rs` (1,698): move the
  closure-body return inference (`try_closure_return_type_for_template`,
  `infer_closure_return_type*`, `resolve_closure_body_type`,
  `walk_closure_body_locals`; 109-187, 422-784) into `closure_returns.rs`
  and the argument-text typing (`resolve_arg_text_to_type`,
  `array_literal_element_type`, `array_literal_shape_type*`; 205-412,
  977-1123) into `arg_text_types.rs`.
- `src/type_engine/types/conditional.rs` (1,642): move the `TextArgs`
  impl (218-729; `resolve` is 285 lines, `class_string_condition` 218)
  into `conditional/text_args.rs` and the no-argument path
  (`resolve_conditional_without_args*`, `try_resolve_with_template_default`,
  `ThisContext`; 1316-1595) into `conditional/without_args.rs`.
- `src/type_engine/call_resolution/return_types.rs` (1,477): move
  class-constant and static-access typing (1019-1270) into
  `return_types/class_constants.rs` and keyword, literal and operator
  typing (1275-1476) into `return_types/literals.rs`. In `call_arms.rs`,
  move the Laravel special cases at the top of
  `return_types_of_function_call` (432-487: the container string binding,
  `now()`/`today()`, `view()`) into `return_types/laravel.rs`.
- `src/type_engine/types/resolution.rs` (951): move type-alias expansion
  (`resolve_type_alias_typed` through `expand_aliases_in_source`, 645-950)
  into `types/type_aliases.rs`.
- `src/class_lookup.rs` (1,025) does two jobs, as its own doc says: lookup
  (27-329) and subtyping (339-1024, with the 383-line
  `is_subtype_of_typed`). Move the subtype checks and the constant
  shape-key evaluation into a module of their own.

## 25. Split the narrowing and forward-walk files

- `src/type_engine/types/narrowing/guards.rs` (1,385): move
  statement-exit analysis (`statements_unconditionally_exit`,
  `statement_leaves_block`, `contains_entry_label`, `ExitCtx`,
  `ReceiverGuard`, the never-call detection; 181-540) into
  `narrowing/exits.rs`.
- `src/type_engine/types/narrowing/assertions.rs` (1,220): move
  conditional-return narrowing (`CallReturnInfo` through
  `can_hold_a_bool`, 967-1219) into `narrowing/conditional_returns.rs`.
- `src/type_engine/variable/forward_walk/by_ref.rs` (1,268): move
  `extract()` handling (529-785) and the built-in by-reference seeding
  (the 237-line `seed_pass_by_ref_primitives` and the `preg_*` helpers;
  922-1238) into `by_ref_builtins.rs`.
- `src/type_engine/variable/forward_walk/assignment.rs` (1,253): the
  walker's statement dispatcher (`process_statement`,
  `process_echoed_expression`, `process_expression_statement`,
  `flatten_namespaced_statements`; 18-344) lives in the assignment file.
  Move it to `forward_walk/statements.rs`.
- `src/type_engine/variable/closure_resolution.rs` (1,117): move the
  Eloquent relation-callback typing (`try_relation_query_override`
  through `extract_model_from_builder`, 742-1085) next to the other
  relation code, and replace the three `*_pub` wrappers (1093-1116) with
  `pub(crate)` on the functions they forward to.
- `src/type_engine/variable/rhs_resolution/property_access.rs`:
  `resolve_rhs_property_access` (25-576) is one 552-line function. Split
  it into its receiver, declared-type and narrowing phases.

## 26. Split the large Laravel and Blade files

- `src/virtual_members/laravel/helpers.rs` (1,476) is a grab-bag, as its
  doc lists. Move the English inflector (`UNINFLECTED_WORDS`,
  `IRREGULAR_SINGULARS`, `singularize_english_word` and helpers; 307-840)
  into `inflector.rs`, the expression walker (`walk_*`, `enclosing_body`,
  `covers`; 981-1135, 1192-1475) into `expr_walker.rs`, and the route
  group prefix helpers (214-301, 843-965) next to `route_names.rs`.
- `src/virtual_members/laravel/mod.rs` (860): move `LaravelModelProvider`
  (370-860, whose `provide` is 363 lines) into `model_provider.rs` and
  split `provide` by the kind of member it adds.
- `src/virtual_members/phpdoc.rs` (1,449): move `@mixin` collection (the
  mixin cache 38-88, `MixinCollector`/`MixinDedup`/`MixinSubs`,
  `collect_trait_mixin_members`, the 311-line `collect_mixin_members`,
  forwards-calls, template-parameter mixins and the substitution maps;
  658-1443) into `phpdoc/mixins.rs`.
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
