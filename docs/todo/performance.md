# PHPantom — Performance

Internal performance improvements that reduce latency, memory usage,
and lock contention on the hot paths. These items are sequenced so
that structural fixes land before features that would amplify the
underlying costs (parallel file processing, full background indexing).

Items are ordered by **impact** (descending), then **complexity** (ascending)
within the same impact tier.

An item belongs here only when it saves at least 7% memory or 4% time
on a realistic use case, or 8% for a High or Very High complexity
change. Trivial fixes are batched and made without being weighed
against that bar.

| Label      | Scale                                                                                                                  |
| ---------- | ---------------------------------------------------------------------------------------------------------------------- |
| **Impact** | **Critical**, **High**, **Medium-High**, **Medium**, **Low-Medium**, **Low**                                           |
| **Complexity** | **Low** (mechanical/boilerplate, no design decisions), **Medium** (self-contained, follows an existing pattern), **Medium-High** (spans modules, some new design), **High** (shared/core subsystem, correctness or performance tradeoffs), **Very High** (cross-cutting architecture, wide blast radius) |

---

## P82. Trivial memory fixes

**Impact: Low-Medium · Complexity: Low**

Measured on the same app. Each is small; together they are about 5% of
the LSP's RSS, and the intern store sweep also bounds growth over a long
session.

- **Symbol map capacity slack.** `spans` carry 17.7 MB of unused capacity
  and `call_sites` about 11 MB. A `shrink_to_fit` when the map is built
  reclaims about 29 MB (3.5%).
- **Member intern store never sweeps.** `substituted_methods` and
  `substituted_properties` in `virtual_members/cache.rs` hold the
  transformed member strongly and are only cleared at 1M entries. An
  edit that re-parses a file leaves its old transformed members alive,
  and each entry's `Weak` witness keeps the dead origin's allocation
  too. Sweep dead-witness entries when the map has doubled since the last
  sweep, as `linked_aliases` already does at its cap. The method table
  alone is 24.5 MB after a full `analyze`.
- **Interner buckets.** `php_type/intern.rs` gives every hash its own
  `Vec<Weak<TypeKind>>`, though almost every bucket holds one entry.
  That is about 114k allocations and 5 MB. Store a single entry inline.
- **Audit drop probes keep table capacity.** `mem_audit.rs` releases
  each store with `clear()`, which keeps the hash table allocated, so
  about 40 MB of table capacity is reported as "residual" instead of
  under its store. Use `std::mem::take`.

---

## Appendix: Profiling

### Commands

```sh
# Flat profile of a whole-project run. The release build has symbols but
# no debug info or frame pointers, so for call graphs build with
# RUSTFLAGS="-C force-frame-pointers=yes" and record with `-g`.
perf record -F 999 -- ./target/release/phpantom_lsp analyze \
  --project-root <dir> --no-colour

# Text report (top functions):
perf report --stdio --no-children --sort symbol | head -80

# Flamegraph (requires the `flamegraph` crate or perf-tools):
perf script | flamegraph > /tmp/phpantom.svg

# Instantaneous CPU utilisation over a run (% of one core, sampled
# every 0.5 s), for machines where perf is unavailable
# (kernel.perf_event_paranoid > 2):
./target/release/phpantom_lsp analyze --project-root <dir> --no-colour \
  >/dev/null 2>&1 & PID=$!; prev=0
while kill -0 $PID 2>/dev/null; do
  cur=$(awk '{print $14+$15}' /proc/$PID/stat 2>/dev/null) || break
  [ -n "$cur" ] && echo $(( (cur - prev) * 2 )); prev=$cur; sleep 0.5
done
```

### Memory

```sh
# Per-store deep-size audit printed at the end of `analyze`. The feature
# installs a counting allocator, so build it into its own target dir to
# keep the normal binary untouched.
cargo build --features mem-audit --target-dir target/memaudit
PHPANTOM_MEM_AUDIT=1 target/memaudit/debug/phpantom_lsp analyze \
  --project-root <dir> --no-colour
```

The audit walks every long-lived store with shared `Arc`s counted once,
then drops each store in turn and reports the exact bytes released. For
an editor session, `LspClient` in `benches/memory_usage.py` can drive the
server (initialize, wait for indexing, open, hover) while you sample
`VmRSS`. The two differ: a large Laravel app settled at 842 MB in the LSP
against a 1,016 MB peak for `analyze`.

Use heaptrack only on small projects (around 100 MB peak), and run it
inside `systemd-run --user --scope -p MemoryMax=8G`. Its bookkeeping
scales with the allocation count, and on a large Laravel app it exhausted
a 64 GB machine.

### Growth checks

A whole-project profile hides a cost that only grows on one unusual
input. Generate the input at N and 2N (a long top-level script, a
`switch` over N distinct literals, N conditional writes to one array)
and compare the two times: doubling N should roughly double the time.
