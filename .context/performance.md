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

**It does not cover the effort-delta reads.** `cube_equivalence` walks specs and
the cube; `effort_metric_deltas` (the effort panel) is a different read path with
its own queries, and the biggest single hotspot ever measured here lived in it
(tsk239, below) while the harness reported a clean profile. A green
`cube_equivalence` is necessary, not sufficient — profile the live app too.

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
- `representative_facts_by_slice` was **38% of backend CPU** in a live capture;
  most calls no longer run it at all (tsk239/tsk242, below).

## Zero-splice producer discovery (tsk239)

The effort panel asks, per measure, "which producers emit this metric's slice"
so a clean run zero-fills instead of reading blank. Answering it by scanning
every fact of the measure cost 38% of live backend CPU — 917k rows scanned to
return 277, with a temp b-tree because the slice key includes the open
`dims_json` TEXT payload, which no index covers.

**The scan is the floor, so the fix is to not scan.** Three branches now, picked
off the spec's filter (`collection.rs`, zero-splice fallback):

| filter | path | cost (917k-fact measure, warm) |
|---|---|---|
| unconstrained | memoized `producers_for_measure` | ~0 (memo hit) |
| reads only `rule`/`severity`/`dims_json` | `distinct_slice_keys` | 0.41 s |
| reads `value`, or a `package`/`branch`/`subject`/`model` dim | `representative_facts_by_slice` | 0.80 s |

The middle branch works because **every fact in a slice agrees on the slice
key**, so a predicate reading only those fields is decided by the key alone — no
representative row needed. `FactFilter::slice_key_only` gates it and
`dim_is_slice_key` classifies the dimensions; that classifier is the negative
image of the match in `dim_value_cached` and nothing but
`every_pseudo_dimension_is_classified_as_slice_key_or_not` holds the two
together. Add a pseudo-dimension there, classify it here.

Of the 25 filtered specs in this project, 22 land on the cheap branch (all the
`dim_eq` ones — `oxplow.rule` is a column and the rest are `dims_json` keys) and
3 on the expensive one (`min_value`/`max_value`).

Still on the table: a covering index on `fact(measure_id, rule, severity,
dims_json, capture_id)` takes the slice-key scan 0.41 s → 0.11 s, but costs
**146 MB against a 323 MB `fact` table** plus write amplification. Filed, not
taken — see the retention argument below.

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

## The renderer is not the problem (at idle)

Every measurement above is of the Rust side. The renderer had never been
profiled at all, so its cost was pure assumption — and the assumptions were
wrong.

Profile it with `tests-e2e/profile-renderer.mjs` (real React UI in headless
Chromium against `oxplow-daemon`; setup in `tests-e2e/README.md`). Two captures,
20–25s each, ~150–190k samples:

| state | idle/program | executing JS |
|---|---|---|
| empty project | 100.0% | **0.0%** |
| 150 tasks, WORK section expanded | 100.0% | **0.0%** |

The always-on timers cost `fetch` 0.01% and `setTimeout` 0.00%. **Do not
optimize a renderer timer because it looks expensive in source.** A 2s poll
doing a trivial thing is free; the interval is not the cost.

Caveats to state whenever quoting these: it's **Chromium, not WKWebView** (good
proxy for JS, poor one for paint/scroll/GC), and it's **idle** — interaction
cost (typing, scrolling a large diff, metric pages against real data) is still
unmeasured.

## Ruled out by measurement — don't redo these

Negative results are the easiest knowledge to lose, and every one of these was
asserted confidently (by me) *before* measuring, and was wrong.

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
- **Folding `representative_facts_by_slice`'s join-back into the `GROUP BY` is
  slower, not faster.** SQLite guarantees that with exactly one `min()`/`max()`
  aggregate, bare columns come from the extreme row — so the group can yield the
  whole representative and the `id IN (SELECT MIN(id) …)` join-back looks
  redundant. Measured: **1.43 s vs 0.78 s**. Dragging 26 columns through the
  group-by sorter costs more than 277 rowid lookups afterwards. The query keeps
  its "redundant" shape on purpose.
- **`SELECT DISTINCT producer, rule, severity, dims_json` is not a drop-in for
  `representative_facts_by_slice`.** tsk239 proposed it as "semantically
  equivalent for the caller" at 4.5×. It isn't: `FactFilter::matches` also reads
  `value` and — through `dim_value` — `path`, `subject_ref`, `subject_kind` and
  `branch`, none of which are in the slice tuple. It's a valid path only when
  the filter provably stays inside the slice key, which is what
  `slice_key_only` checks. The measured win on that guarded path is ~2×, not
  4.5×.
- **The renderer's idle timers cost nothing.** Three suspects were filed off a
  source read: the 2s daemon-recovery poll, a 1s agent watchdog, and
  `BrailleSpinner`'s 80ms interval "re-rendering the task list at 12.5 Hz". The
  first two measure at 0.01% combined. The third was wrong on inspection, not
  measurement: `BrailleSpinner` holds its own `useState`, so the interval
  re-renders a self-contained leaf `<span>` — it cannot re-render its parent.
  (It stays formally unmeasured: mounting it needs `agentStatus === "working"`,
  and no write command sets agent status.)

## Daemon transport: what a loopback HTTP hop costs (tsk255)

Measured for the daemon-backed shell epic ([[tsk254]]), from **inside the real
WKWebView** with embedded assets (the packaged path), against a local
`oxplow-daemon`. The comparison that matters is Tauri IPC vs daemon HTTP — *not*
"in-process vs network" — because today's local path is already JSON over
Tauri's IPC bridge, not a function call.

| call | Tauri IPC | daemon HTTP |
|---|---|---|
| `ping` (no work) | p50 ~0 ms, p95 1 ms | p50 4 ms, p95 5 ms |
| `list_workspace_files` | p50 2 ms, p95 5 ms | p50 5 ms, p95 5 ms |
| `GET /health` (no preflight) | — | p50 3 ms, p95 9 ms |

So **~3–4 ms per call**, and it is *not* CORS preflight: a plain GET with no
custom headers costs the same as a JSON POST, so the floor is the WKWebView
networking-process hop itself. `performance.now()` in this webview quantizes to
~1 ms, so treat these as coarse.

Two things keep that from being alarming, and one that should shape the design:

- The benchmark is **sequential**; a real page load issues its calls
  concurrently, where the per-call latency overlaps rather than sums.
- Terminal keystrokes pay it twice (input + echoed output) — ~8 ms against a
  ~100 ms human inter-keystroke interval.
- **The `/events` WebSocket is already open and does not pay the per-request
  hop.** Running RPC over that socket instead of `POST /ipc/:name` is the
  designated optimization if the HTTP floor ever bites; don't rebuild the
  transport before measuring that it does.

**CSP is load-bearing here.** With embedded assets and the shipped policy, the
webview cannot reach the daemon at all — `fetch` → `TypeError: Load failed`, the
WS constructor throws. Adding `http://127.0.0.1:* ws://127.0.0.1:*` to
`connect-src` makes both work; loopback only, no ATS wrinkle on macOS. In **dev**
the page is served by vite over http, so Tauri never applies the configured CSP
and the restriction is invisible — a dev-mode test of this proves nothing.

## Related

- [metrics.md](./metrics.md) — the metric substrate itself: the cube, its two
  counters (`epoch` fences writers, `version` invalidates the read cache), the
  event-scoping and debounce stack, and the retention/compaction knobs.
- [data-model.md](./data-model.md) — the DB pool and `spawn_blocking` cap.
