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

## oxplow-bundled's co-change pairs (P7.B5, 2026-10-02)

The plan moved co-change out of core only if its SQL could be
materialized in under 2 s on this repo's commit index. Measured with
sqlite3 on this repo's real history (`git log --all --since=400.days`:
2,351 commits, 19,336 file rows — the dev database here predates the
commit index, so the rows were loaded from git): `co_change_pair`'s
SELECT runs in **0.14 s** (18,392 pair rows, both ways round). It's
`materialize: on_change`, refilled when the commit index moves; a
change's surprises (`change_co_change`) are a live view over it, the
last-touch lookup using `idx_git_commit_file_path`.

## oxplow-bundled's deviation model (P7.C5, 2026-10-02)

`v_oxplow_bundled_deviation` (an extension model with a recursive CTE over
every effort file's directory prefixes) computes all efforts at once:
**0.21 s** over this repo's 658 efforts / 5,484 effort files (sqlite3 on
the live database, the same SQL over the physical tables). It stays a
live view; `materialize: on_change` would recompute on every task edit
for a read that's cheap enough as is. Revisit when a What Deviated slot
read shows up in a profile.

## The cube decision at 7.1 M facts (P4.4, tsk489, 2026-09-29)

Measured before `metric_grid()` (P4.5) builds on the engine: the harness on a
`VACUUM INTO` copy of the live project DB (2.4 GB, **7,097,464 facts**, 47
measures, 73 specs), cube cleared, release build. The harness now prints
per-spec times and reads the cube pass through a fresh engine, so no
in-memory state from the fact pass can pass for the cube.

| | |
|---|---|
| All 73 specs fact-served (the oracle) | **68.3 s** |
| Cube build (58,232 captures folded, one-time; then incremental) | 53.9 s |
| All 73 specs with the cube (61 cube-served, 12 declined to facts) | **1.6 s** |
| `oxplow.fn_count` | 23,938 ms → 1.3 ms |
| `oxplow.tests.duration_ms` / `.passed` / `.failed` / `.total` | 6,880–11,356 ms → 11–13 ms |
| `oxplow.coverage.abs_pct` | 827 ms → 27 ms |
| the 12 declined specs (`long_functions`, `high_complexity_fns`, `coverage.untested_files`, …) | ~same both passes (270–380 ms) |

Every series was identical to the oracle. **Decision: keep the cube** as the
engine's accelerator behind `metric_grid()`; it wins by ~44× overall and by
three to four orders of magnitude on the specs a person actually opens. The
declined specs are the next place to look, not the cube.

**The size is per-case test facts:** `oxplow.test_case` (3.16 M) and
`oxplow.test_duration` (3.06 M) are 88% of all facts — retention or
aggregation of those is its own task (tsk514; see the next section for what
they cost the cube).

## Bounded series reads, re-measured (P12, tsk202, 2026-10-05)

The harness on a `VACUUM INTO` copy of the live DB (2.3 GB, **5,969,753
facts**, 108,667 captures folded in 73.8 s), release build: every series
identical to the oracle, **309,164 ms fact-served → 3,411 ms with the cube**
for all 73 specs.

| | ms |
|---|---|
| a cube-served spec | 0.4–75 (most 1–30) |
| `oxplow.long_functions` (declined to facts) | 698 |
| `oxplow.high_complexity_fns` (declined) | 685 |
| `oxplow.coverage.untested_files` (declined) | 544 |

So tsk202's shape — a series loading a measure's whole history for the
renderer to filter — is gone: reads take a window, applied server-side, and
the cube's whole-history read for the largest measure (`oxplow.tokens`, 45 k
cube rows) is ~10 ms in SQLite. **Pushing the window into the cube's SQL
isn't worth doing**: it would save at most that. More than half of what's
left is three per-path *threshold counts* the cube declines; a time window
can't cut them — an in-window point still folds per-path state from before
it. They need per-capture counts built with the cube (tsk1019).

## The cube as an asset: what a burst costs (P7.B1, 2026-10-01; corrected 2026-10-02)

`crates/oxplow-app/examples/cube_burst.rs` on a `VACUUM INTO` copy of the
live project DB (3.8 GB, ~11.3 M facts, 47 measures), release build. The
asset runner calls `build_all` once per quiet burst of commits to
`metric_capture` / `fact`:

| | |
|---|---|
| A burst with nothing new to fold | **~190 ms** (47 measures' watermark checks) |
| A burst after a test run (2,598 per-case facts) on a branch the cube holds | **~0.9 s** — `oxplow.test_case` 332 ms, `oxplow.test_duration` 145 ms |
| The same run on a branch the cube has never seen (that branch's **seed**) | **~25 s** — 15.8 s + 8.7 s for the two per-case measures |

**The first version of this table said every test run cost ~22 s. It
didn't:** the harness recorded its one capture with no branch, so it timed
a new branch's seed, not an incremental fold (tsk704). A test run lands on
its branch's existing partition and folds in well under a second.

**The seed is the real cost.** A branch's first fold replays the history
visible to it into its live partition, and for the per-case test measures
that is every one of ~5 M facts. It is paid once per new branch, and again
for every branch after anything invalidates the cube (the dominated-capture
prune — no longer on a clean tree's `oxplow.duplicate_lines` restate,
tsk709 — or a promoted-dim flip). A per-subject seed is now one SQL query
over the visible captures (`live_seed_per_subject`: the latest capture per
`(producer, subject)`), equivalent to the replay and pinned against the
fact fold, instead of loading every fact into memory; that took the seed
from 25.3 s to 20.0 s. The query alone is 17.8 s on 5 M facts, and a
covering index `fact(measure_id, capture_id, subject_ref, path)` only
brought it to 8.2 s for a large index — **the lever is the volume of
per-case facts** (tsk514), not the fold.

**Where the facts are:** `oxplow.test_case` and `oxplow.test_duration` are
87.5% of all facts (the `fact` table is 1.9 GB, its indexes another
1.4 GB). 1,979 of the 2,044 `tests` captures are effort-stamped, which the
opt-in `metricRetentionDays` keeps unconditionally — so no retention
setting reclaimed them.

**Change-only test facts (tsk733)** replace the per-case window tried
first (tsk514, a 7-day prune, removed: on this machine 3.9 M
`oxplow.test_case` facts sat inside a single week, ~210 runs a day, so a
window bounded history but not volume). Measured on a fresh copy (tsk732,
sample usage — 2,302 runs, 5.3 M case results, 99.8% passes): 0.25% of
results changed status from the test's previous result on the branch,
2.1% moved duration more than 50% and by at least 20 ms, and 34% of runs
exactly repeated the previous run's results. Recording failures always and
passes or skips only when new, flipped or past the duration tolerance
keeps **~2.6%** of per-case results; per-test history lives in
`test_case_stat` (12,859 rows for that whole history: one per branch ×
test). A seed reads what's left.

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

## Extension catalog cache (tsk390 / P1.13, tsk415)

`extensions::load_extensions(root)` parses every bundled lens file (about
forty embedded YAML documents) plus the project's on each call, and it ran
on every advisory check (each agent tool call and prompt), every
`list_extensions` / `get_lens` / `run_lens`, the schema listing and metric
catalog seed. Measured on the dev machine (release-less test build, 20
calls averaged):

| call | per call |
|---|---|
| `load_extensions`, bundled only | ~3.3 ms |
| `load_extensions`, bundled + one project extension with 10 lenses | ~3.1 ms |
| `ExtensionCatalog::get`, same root, cache hit | ~62 µs |

`crates/oxplow-app/src/extension_catalog.rs` caches the loaded `Vec<Extension>`
per worktree root behind a **stat-only fingerprint** of
`root/oxplow/extensions/**` and `root/.oxplow/project.yaml` (path, size,
mtime of every file). A hit walks the tree with `stat` and parses nothing;
any edit, add or delete — or a config change that disables an extension —
misses and reloads on the next call. No watcher and no explicit
invalidation, so there is no window in which an agent's freshly written
lens is invisible, and worktrees the watcher doesn't cover behave the
same. Write paths (install, update, `lens.keep`) read the disk directly, and
consent hashing (`approval_hash`) always reads the bytes it approves.

## Metric visibility: ancestry in bulk (2026-10-06)

Every boot pegged one thread for 8–12 minutes in
`metric_visibility::resolve`: it asked git about each (absorbing commit,
base commit) pair with its own `graph_descendant_of` walk, and this
repo's captures have ~860 distinct anchors on each side — hundreds of
thousands of walks, under the oracle mutex every other metric reader
waits on. `RevisionGraph::ancestry` now answers the whole cross product
from one parents-first revwalk that ORs each commit's parents' bitsets
of anchors (`vcs/git.rs`), and `GraphOracle` asks the graph once per
resolve for the pairs it hasn't cached. Measured on this repo: 1,078
anchors × 1,078, 1.16 M pairs over 2,156 commits, in 1.2 s in a debug
build. Don't go back to per-pair questions; the cross product is not
"tiny by construction".

## Effort evidence after the cube, through the shared engine (2026-10-06)

Right after boot the daemon went from ~150 MB to 1.7–2.2 GB: the
`effort_evidence` asset's first build raced the cube's backfill, so every
per-path effort delta missed the cube (`cube_series` serves only when the
watermark covers every capture) and folded the measure's whole history —
on a per-call `MetricEngine` whose fold memo was always empty. Evidence
now reads the cube asset as its input (it recomputes after the cube,
never while it's pending; `assets.rs`), and `CollectionService` uses the
app's one engine. A spec with `min_value`/`max_value` still can't be
cube-served; it folds once per capture token, shared.

## The dev loop: build and test time (tsk678, 2026-10-01)

Measured on the 14-core dev Mac (under background load ~11, so expect
±30% run to run), for a one-line body edit in `oxplow-app`:

| Step | Before | After |
|---|---|---|
| Coverage-instrumented test build (`cargo cov`'s compile) | 16–18 s | same |
| Rust test execution (nextest, every test its own process) | 47–75 s | 22–31 s |
| Coverage report (`cargo llvm-cov report`) | ~5 s | same |
| Desktop tests (`test:junit`) / typecheck | ~15 s / <1 s | same |
| Clippy (`lint:collect`, its own target dir) | ~23 s after the run | overlapped with it |

**What the time was.** Rust test execution was throughput-bound, not
latency-bound: ~1,045 test-seconds over 14 cores. ~2,000 tests each built
a fresh database, and `Database::in_memory()` ran 128 migrations plus the
core model compile — **~330 ms per test**, in its own process under
nextest, so no in-process cache could help. Now it restores a migrated
template from `<temp>/oxplow-db-templates/<build+date>.sqlite`
(`database::load_migrated`: keyed by the test executable's path, size
and mtime and today's date, written once per build — migrated in memory,
then copied out with SQLite's backup API, since a migration's
journal-mode switch is refused on a file inside its transaction — and
restored with `Connection::restore`; templates older than a day are
pruned). Test-seconds fell to ~320; `oxplow-app`'s median test went
0.36 s → 0.05 s. And two `daemon_supervisor` tests took 30 s about half
the time, setting the suite's floor: a real bug — `stop` asked `getpgid`
of a daemon leader that could already be a zombie (macOS then fails it),
so the SIGKILL spared a child still holding stdout. The supervisor now
puts the daemon in its own process group itself and signals `-pid`
(`signal_group`); `kill_orphan_daemon` keeps the cautious check for a
daemon it didn't start.

**What it isn't** (measured; don't redo): the **linker** (0.8 s of a
14 s `oxplow-app` test-lib build — lld/mold won't help); **incremental
compilation** (works: a body, private-fn or pub-fn edit recompiles
`oxplow-app`'s test lib in ~5 s alone; inside the workspace build its two
units take 6–10 s each in parallel, then dependents ~6 s each — an edit
that adds public API rebuilds rpc/mcp/tauri-ipc too); coverage
instrumentation (its build costs about the same as the plain one);
**splitting `oxplow-app`** (not worth it at these numbers);
**Cranelift / `-Z threads`** (nightly only, and the coverage build needs
LLVM anyway). `cargo cov` clears old `.profraw` files at start; only
`--no-report` runs leave them behind.

**Run lint alongside tests.** `lint:collect` and `test:collect` use
different target dirs, and clippy mostly waits on one crate, so running
them together cut a close from ~174 s to ~115 s:
`(bun run lint:collect >/dev/null 2>&1 & bun run test:collect; wait)` —
still one foreground command, so collection sees both. Read
`target/clippy.json` (or rerun plain clippy) when it fails.

## Related

- [metrics.md](./metrics.md) — the metric substrate itself: the cube, its two
  counters (`epoch` fences writers, `version` invalidates the read cache), the
  event-scoping and debounce stack, and the retention/compaction knobs.
- [data-model.md](./data-model.md) — the DB pool and `spawn_blocking` cap.
