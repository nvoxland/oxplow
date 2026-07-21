# Performance: how to profile, what's already fixed, what isn't worth doing

What this doc covers: how to get a *trustworthy* CPU profile of the metric
path, the shape it has today, and the things measurement has already ruled
out. The point is that re-deriving any of this is expensive and the traps
below fail **silently** — a wrong profile looks exactly like a right one.

Detailed capture-by-capture numbers live on the `metric-path-db-contention`
wiki page. That page is in `.oxplow/wiki/`, which is **gitignored** — so
anything that must survive a fresh clone belongs here, not there.

## The harness: `cube_equivalence`

`crates/oxplow-app/examples/cube_equivalence.rs` runs the whole metric path
against a **real database copy**: every catalogued spec fact-served (the
oracle), then a full cube build, then the same reads again — and asserts the
two agree exactly.

That makes it both the profiling harness *and* the correctness gate: any
metric-path optimization must leave all series identical to the fact oracle.
Use it for both. Every optimization below was verified this way.

```sh
sqlite3 .oxplow/local.sqlite "VACUUM INTO '/tmp/cube-eq.sqlite'"
# The example asserts a PRE-BUILD cube. A copy is already at the current
# migration version, so opening it runs no migrations and clears nothing —
# clear the cube tables by hand or the assert fires:
sqlite3 /tmp/cube-eq.sqlite \
  "DELETE FROM metric_cube; DELETE FROM metric_cube_state; DELETE FROM metric_live_fact;"
CARGO_PROFILE_RELEASE_DEBUG=true CARGO_PROFILE_RELEASE_STRIP=none \
  cargo build -p oxplow-app --example cube_equivalence --release
samply record -r 200 --save-only --no-open --unstable-presymbolicate \
  -o /tmp/prof.json.gz -- ./target/release/examples/cube_equivalence /tmp/cube-eq.sqlite .
```

**It is deliberately sequential** — `build_all` loops measures one at a time
and the example loops specs one at a time. So it measures per-call cost well
and **cannot reproduce concurrency effects at all**. Any claim about pool
contention or lock waiting needs a different harness that fires many
concurrent reads (the renderer's tile fan-out is the real-world shape).

The old recipe of launching the Tauri app under samply and Cmd-Q'ing it still
works, but it can't run while another instance holds `.oxplow/instance.lock`,
and **attach mode freezes the app** — always launch, never attach.

## Two traps that silently produce a wrong profile

**1. Rank by `threadCPUDelta`, not sample count.** samply samples parked
threads too. By raw sample count a metric-path profile reads as ~90%
`__psynch_cvwait` — that is threads *sleeping*, not CPU. Weighting by
`samples.threadCPUDelta` (µs) gives a completely different and correct
ranking. A historical "43% mutex contention" figure on the wiki page is
suspect for exactly this reason.

**2. `--unstable-presymbolicate` is not optional.** Without it the saved
profile has raw addresses and `nativeSymbols` is empty; you get a top frame
of `0x450c`. The flag writes a `.syms.json` sidecar to resolve against.

## The shape today

After the tsk129 and tsk191 rounds, **there is no dominant hotspot left.**

| | |
|---|---|
| SQLite VDBE + `pread`/`pwrite` + `memmove` | ~69% |
| `row_to_fact_row` (the other ~25 columns materializing `String`s) | ~6.9% |
| `dim_value` | 4.1% |
| `tree_state_series::apply` / `fold_series` | ~3.6% each |

Already fixed — **do not re-optimize these**:

- `producers_for_measure` was **46% of backend CPU**; memoized (tsk130/tsk153)
  and now absent from the profile entirely.
- `dim_value` 11.5% → 4.1% (tsk214).
- `string_to_ts` 2.5% → off the board (tsk215).

The remaining ~69% is genuine page reads of real data, so it scales with
database size. That makes **retention the lever, not micro-optimization** —
see the compaction knobs in [metrics.md](./metrics.md).

## The pattern behind every win

All of them were the same bug in different clothes: **recomputing a value
that belongs to a coarser grain.**

- `producers_for_measure` — a property of the *measure*, recomputed per call.
- `dims_json` — a property of the *fact*, re-parsed once per dimension lookup
  (3 promoted JSON-backed dims meant 3 parses per fact).
- `captured_at` — a property of the *capture*, re-parsed per *fact* row
  (~130 facts per capture ⇒ ~130 identical parses).

When something looks hot, ask what grain the value actually belongs to before
optimizing the computation itself. The memo key is then usually obvious and
provably correct: `capture_id → captured_at` is a function, so memoizing on it
is right **regardless of row order** — ordering only affects hit rate.

## Ruled out by measurement — don't redo these

Negative results are the easiest knowledge to lose, and both of these were
asserted confidently (by me) *before* measuring, and were wrong.

- **Storing timestamps as epoch-ms integers is not worth doing for parse
  cost.** It was the headline proposal of the row-decode task. The per-capture
  memo already removed `string_to_ts` from the profile, so the migration would
  buy nothing while costing a rewrite of the `captured_at BETWEEN` string
  comparisons the windowed reads rely on. If it's ever done, do it for
  comparison/index reasons — not parsing.
- **A large WAL is not a read cost.** A 169MB WAL held only **226 live
  frames**; readers index live frames, not file bytes. WAL size is a
  disk-footprint artifact of the biggest write burst (the automatic checkpoint
  is PASSIVE — it restarts the log in place and reuses the space). The daily
  pass truncates it (tsk216), but that is housekeeping, not speed.

## Related

- [metrics.md](./metrics.md) — the metric substrate itself: the cube, its two
  counters (`epoch` fences writers, `version` invalidates the read cache), the
  event-scoping and debounce stack, and the retention/compaction knobs.
- [data-model.md](./data-model.md) — the DB pool and `spawn_blocking` cap.
