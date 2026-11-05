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

## Open-effort evidence (2026-10-16)

An effort open for days holds thousands of token reports (`otel-tokens`
writes one per live agent about every 30 s; two efforts here held 13,459
and 9,437 captures and 315,686 facts). The `effort_evidence` asset
recomputes every open effort when the cube moves, and each report moved it,
so each report meant reading the effort's whole history: `sample` put the
daemon at 50–120% of a core with no turn in flight, in
`refresh_effort_evidence` → `captures_for_effort` + `facts_for_captures`
(`row_to_capture`, `fact_row_mapper`).

What a recompute reads now:

- **The effort's captures as groups** — `effort_capture_groups`: per
  producer and trigger, the count and the newest id. A spec's memo key holds
  the groups of the producers it reads, so a capture of any other producer
  (all of an agent's work is stamped with the effort) leaves it as it was and
  reads no rows.
- **A `sum` spec folds** (`SumFold`): the total, the fact count and the
  newest point of the reports covered so far, with how many there were. A new
  report is read alone (`effort_captures_after`) and added; the fold starts
  over if a covered capture is gone (the count before the old newest id no
  longer matches) or the spec or the effort's window changed.
- **Observations and coverage read only the effort's run captures**
  (`effort_captures_by_trigger`), not every report.

Measured (release, `a_long_open_effort_recomputes_in_milliseconds`,
13,000 token reports of two facts each): reading the effort's captures and
facts again **57.6 ms** (what each recompute did before, per effort, with
twice the facts on the live database); a whole recompute after one more
report **6.7 ms**. Pinned by `a_token_report_folds_into_the_total_without_
rereading_the_effort` (one report reads one capture and two facts, and the
fold equals a read from scratch) and `a_capture_nothing_reads_costs_no_reads`.

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

`v_oxplow_bundled_deviation` (an extension dataset with a recursive CTE over
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
it. They get per-capture counts built with the cube ("Threshold counts from
the cube" below).

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
covering index `fact(measure_id, capture_id, subject, path)` only
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

**A write extends the slice memos instead of dropping them** (measured
2026-10-09). `distinct_slice_keys` and `representative_facts_by_slice` are
memoized per measure, but every write of the measure used to discard them,
so the next read paid the whole `GROUP BY` again — at boot and through agent
activity. Facts are append-only between prunes, so a write can only *add*
slices, and a slice's representative (its lowest-id fact) can't be displaced
by a later one. Each memo now records the highest capture id it covers; a
write keeps it, and the next read looks only at the facts of captures after
that id (`idx_fact_measure_capture` seeks the range) and merges them. A prune
still forgets everything (`invalidate_all`). A read that raced a write
declines to store its result, as before, and the memo it extended stays
valid. 200,000 facts over 400 captures and ~300 slices, a one-fact write then
both reads (`fact_store::tests::slice_read_after_write_cost`, release):

| | Median |
|---|---|
| Before (rescan after a write) | 6,552 µs |
| After (extend from the new capture) | 125 µs |

`an_extended_slice_memo_equals_a_fresh_scan` holds the extended memo to a
fresh scan after every write of a sequence.

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

## An effort's metric deltas, memoized by what each spec reads

`CollectionService::effort_metric_deltas` computes one delta per metric
spec for an effort; the `effort_evidence` asset recomputes them whenever
the metric cube moves, which an agent's token report (an `otel-tokens`
capture every 10–15 s, stamped with the open effort) does. `delta_memo`
skips a spec whose inputs didn't move, so the key (`delta_inputs`) must hold
only what the spec's delta can read: the spec, the effort's span, stream and
claimed files, the capture token of the producers of its source measure, and
**the effort's own captures of those producers** (the run and window
families read their facts; the file family matches the effort's id on those
producers' captures) — the coverage family instead its test-run captures. Keying
on *all* the effort's captures made each token report invalidate every
spec, including the file family, which reloads the producers' whole capture
history and two whole-repo captures' facts: 10–20 s of CPU per recompute in
a debug build, back to back. `an_effort_stamped_capture_recomputes_only_its_producers_specs`
holds it: another producer's capture stamped to the effort recomputes
nothing; one of the spec's own producers does.

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
  `value` and — through `dim_value` — `path`, `subject`, `subject_kind` and
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

## A switch during startup waited on the write lock (measured 2026-10-17, tsk1422)

**Symptom.** For the first minutes after a restart the person couldn't
switch stream or thread; the window's priority lane didn't help.

**Method.** A copy of the live database (`sqlite3 .backup`, 1.3 GB, 206 k
`page_ref` rows) under a debug `oxplow-daemon` with the page-ref repair and
the search assets forced to rebuild, and a probe calling `switch_stream`
and `list_streams` once a second. Reads (`list_streams`,
`get_thread_state`, `query_sql`, `list_extensions`) stayed at a few ms the
whole time at 100–130% CPU. **`switch_stream` — a write — took up to 4.2 s**
(the first run, before any change) while the startup work ran.

**The wait.** A write waits for SQLite's one write lock. Its holders
during startup are the snapshot take, the page-ref repair's batches, asset
rebuilds and the event pump's consumers, each holding it for 0.3–1.2 s (a
debug build); behind them queue dozens more writers, and SQLite's busy wait
isn't a queue: a waiter wins the lock when its poll happens to land after a
release, so a switch lost to writers that arrived after it, one hold after
another. Every queued writer also held one of the pool's 8 connection
slots, so a request that needed a slot queued behind them. It isn't CPU
(reads were fast), a generation pin or an asset lock. Found with a log of
slow writes (`slow database write`, with the caller's file and line, the
time it waited for the lock and the time it held it; either past 250 ms).
A caller whose one line serves many things names what it wrote:
`Database::transaction_labeled(label, …)` logs `caller=file:line (label)`;
the search index's restate says `(search kind <kind>)`.

**Fix.** `Database` knows which work a person waits on:
`oxplow_db::interactive(…)` scopes a request (the daemon's `/ipc` handler
wraps every call). In that scope a write transaction is counted from the
moment it asks, and every background writer waits for the count to reach
zero before it takes the lock (at most 1.5 s, so a steady stream of
interactive writes can't starve it). Background work also takes one of
`pool − 2` slots before the gate, so two connections are always the
person's. Any write a request makes counts, whichever API it goes through
(the busy handler counts a request's call as it starts to wait for the
lock); a background writer yields, then waits up to 30 s, so it is never
refused for it. Opening the database also retries a locked file (the
instance being replaced, still winding down; this was the cause of
`a_restart_replays_an_idempotent_job_that_was_running` failing with
"database is locked" in a full run: its second `Database::open` met the
first instance's writers). `switch_stream` is interactive; it now waits at most for the one holder that has the lock
(0.5–0.7 s in the same debug repro, from 2.4–4.2 s). A background writer
that holds the lock longer than a second still delays a switch by what is
left of its hold; the repair already writes in batches
(`page_ref_backfill`), the snapshot take is one transaction by design.
Pinned by `an_interactive_write_goes_ahead_of_queued_background_writes`
(a 700 ms holder and twenty queued writers: the interactive one waited
2.9–4.0 s without the yield, under 1.3 s with it).

## The window's stall detector reports only a window that was drawing (2026-10-17, tsk1423)

WKWebView has no long-task observer, so `logger.ts` detects a frozen main
thread by timer drift: a tick every second, and a `main thread stalled`
WARN when one arrives a second or more late. macOS throttles a covered or
background window's timers to about one every 2 s and still reports its
document visible, so every tick of such a window looked like a ~1000 ms
stall: 7,417 WARNs on 2026-10-16, one every ~4 s for hours, each with a
`blockedMs` of 1001–1194. The detector now also asks whether the window was
drawing (`framesRanSince`: a `requestAnimationFrame` frame ran no earlier
than a tick before the previous tick); the browser runs no frame callbacks
for a window it isn't drawing, while a real block starts after frames were
running. A hidden document (`visibilitychange`) is still discounted. At
most one tick right after a window is covered can still report.

## A worktree's files as a live dataset (measured 2026-10-18)

`v_file` (on the local filesystem) reads a stream's worktree on every query: the VCS
status, the filtered walk, and the rows into a temp table. On this repo
(2,321 files kept by the filter): status ~28 ms, walk ~40 ms, temp table
~2 ms — about 70 ms a query, and the walk grows with the tree. Listing one
folder (what the old `list_workspace_entries` IPC did) costs the status and one
`read_dir`, about 30 ms. So a query that names one folder (`WHERE parent =
…`) reads that folder alone (`live_datasets::pushdown`, semantic-layer.md
"Live datasets"): the status and one `read_dir`, as the IPC. The Project
panel, Quick Open's file index and the Directory page can then read the
dataset; the whole-tree walk is left to the queries that need the tree.
`v_file` fills `size` and `modified_at` only for a one-folder listing (one
`stat` per file, 16 at a time, at most 1000 files); the whole-tree walk
leaves them NULL so Quick Open and the link index pay nothing for them.

### An edit outside oxplow reaches the Uncommitted panel at once (2026-11-01)

The panel read `v_change_file`, restated only on a ref or index move or
after a snapshot take: a take waits for a quiet worktree (3 s) and then
queues behind the pump's other consumers, which measured 25-30 s on a
throwaway clone. Not reproduced in the real app here (no display);
the fix removes the wait rather than tuning it: the stream's file watcher
(250 ms debounce) now tells the working-files listener on each event, and
queued moves are listed once per stream
(`an_edit_on_disk_tells_the_working_files_listener`). Expected delay:
the debounce plus one git status of the worktree. A busy worktree (a
build writing tracked files) now relists about once per debounce window,
not per event.

### The Measure data page reads recent captures first (measured 2026-11-01)

`slow page re-run page=oxplow-foundation/measure-data loaders=facts took_ms=646..849`.
Measured on this repo's live database with the bundled SQLite (3.51;
the system `sqlite3` CLI plans differently, so do not time with it), for
`oxplow.tokens` (138,000 facts, 153,000 `v_fact` rows, 130,000 captures):

| the facts loader's query | time |
|---|---|
| `measure_key = K ORDER BY id DESC LIMIT 500` | 790-940 ms |
| the same with the page's default 30-day `captured_at >= …` | 660-690 ms |
| the same with a 4-day `captured_at` bound | 170 ms |
| `capture_id >= (a bound 9,000 captures back)` | 58-66 ms |

The plan was already right (`SEARCH ch/ph USING INDEX … (measure_id=?)`;
reordering the chain joins gave nothing). The cost is reading every fact
of the measure in range through five joins to throw all but 500 away: a
`captured_at` bound is tested after the joins, while a `capture_id` bound
is a range on `idx_fact_measure_capture`. So the loader now takes the
newest 500 from the last day's captures, then the last week's, then the
whole range, each tier skipped (a constant guard on the earlier tiers'
counts) when a narrower one held 500; newest first is by capture, then
fact id (held facts keep old ids, so a window by capture is exact only
ordered by capture). Pinned by the page's fixture (same rows); the
timings above are not asserted.

Done after: a page that reads `v_fact` re-ran when any measure's facts
landed, since reads were recorded per dataset and only `metric_grid()` reads
named measures. A loader now filters on `measure_key = read_measure(<key>)`
(a SQL function that records the key, `sql_functions.rs`): the run is
invalidated by facts landing for that measure only (`Reads.measure_scoped`;
pinned by `a_run_reading_one_measures_facts_ignores_the_facts_of_others`).
Used by measure-data (and its `measure_branches` loader) and
dimension-data; the metric
data page is not converted. The hidden-tab pause (above) removes the cost of tabs not on screen.

### `v_dataset` no longer follows the assets (measured 2026-11-01)

`v_dataset` joined `asset_state` for `computed_at`, `events_to` and
`mode`, which every asset recompute writes, so each page reading it
(data catalog, dataset definition, Dataset data, resource data) re-ran
whenever anything recomputed: Dataset data's `about` loader every 3-5 s
while agents worked, `slow page re-run … loaders=about took_ms=281..1075`
in the daemon log. Now `v_dataset` is the registry alone (v2; the freshness
is in `v_asset`). Pinned by
`the_dataset_catalog_does_not_read_what_the_assets_write`. The view is not
slow itself: on this repo's live database `SELECT count(*) FROM v_dataset`
(165 rows, 75 KB of SQL text) takes about 30 ms in the sqlite3 CLI. The
300-1000 ms a run took was waiting: the same minutes show the 380-935 ms
search restates holding the write lock (see the next entry), and a run
queues behind it.

### A search index restate no longer holds the write lock to read (measured 2026-11-01)

The daemon log showed `slow database write caller=crates/oxplow-app/src/kind_search.rs:105
held_ms=380..935` (the 935 ms at startup), with other writers
(`event_pump.rs`) waiting up to 1.1 s behind it. `KindSearchIndex::recompute`
read the whole feed (a kind's view, text and all) and compared it inside
one write transaction, so the lock was held for the read. Now the feed
is read and the diff worked out in a read transaction (no write lock);
the write transaction opens only when the plan has something to write,
re-plans against the stored entries (no feed read) and writes. An index
that is up to date takes no write lock. Pinned by
`a_recompute_takes_the_write_lock_only_to_write_a_change` (a file
database: the in-memory one has one connection, so every read waits).
The figures above are from the log before the change; they were not
re-measured live. Note for tests: background work shares all but two of
the pool's connections, so a *background* writer holding the lock blocks
a background read too; hold the lock as `oxplow_db::interactive`.

### Hidden tabs do not re-run (measured 2026-11-01)

`CenterTabs` keeps every tab body mounted (`display: none`), so a tab
the person left still held an open run, and the daemon re-ran it on
every change to a dataset it read. A background "Agent tokens data" tab
(`measure-data`, loader `facts`) re-ran every ~2 s at 0.8-1.3 s a run
while agents wrote token facts (`slow page re-run` in the daemon log),
and the daemon sat near 97% CPU; a hidden `dataset-data` tab did the
same. Fix: a run with no host on screen is marked stale and not
re-run; it re-runs once when its tab shows (`PageRuns::set_visible`,
pinned by `a_hidden_run_waits_for_its_tab_to_show_and_then_re_runs_once`
and the store's test in `pageRuns.test.ts`). The cost of a tab you do
not look at is now zero. A visible slow page is a separate matter (the
Measure data page, below).

### An async consumer's skipped events are one checkpoint write (measured 2026-11-01)

The daemon log after a restart held 250 `slow database write` lines in 44
seconds; 140 were `event_pump.rs` (the checkpoint write at the end of
`deliver_async`), all pure waits (`waited_ms` 390–2516, `held_ms` 0). Every
async consumer (21) reads every event and wrote its own checkpoint in its
own transaction, also for an event it has no handler for. One event was
21 write-slot requests, and at start-up each queued behind the backfills
that hold the lock (`page_ref_store`, `fact_store` rebuilds, 0.7–1.9 s a
hold). The 32 + 32 lines at `assets.rs:207` / `:821` are one `track` and one
`asset_state` write per asset at start-up, also waits (`held` 0–7 ms).

`run_slot` now moves the checkpoint over a run of events the consumer does
not handle in one write (at the end of the batch, or before a deferral); a
handled event's own checkpoint covers the skipped ones before it. Pinned
by `events_an_async_consumer_skips_move_its_checkpoint_in_one_write`
(ten skipped events: ten writes before, one after). Not changed: the
holders, and the per-asset start-up writes. Live effect is not measured: the
acceptance is under 20 slow writes from these callers in five minutes of an
active session after `run:dev`.

### Boot write holders: the dominated-capture prune (measured 2026-11-01)

After the checkpoint fix, the first minute of a boot still logged
`fact_store.rs prune_dominated_tree_captures` holding the write lock 2.8 s
with four writers queued behind it. Measured on a copy of the live database
(1.96 GB, 131k captures; bundled SQLite): the prune deleted **nothing** (0
captures doomed in every stream), the whole hold was working that out, inside
the write transaction. Its cost was the join from each stream's newest full
capture per producer (~45 of them) to that producer's captures, which had the
stream's index alone: each producer scanned all of the stream's 103k
captures (stream 1: 1.50 s, stream 2: 0.42 s). With an index on
`(stream_id, producer, captured_at)` (a migration) the same read takes
0.22 s / 0.03 s; and the read now runs on a read connection, taking the write
lock only when something is doomed (a capture stays dominated once it is, so
the ids read are still doomed when the write lands). Pinned by
`a_prune_that_finds_nothing_does_not_wait_for_the_write_lock` and
`captures_are_indexed_by_stream_and_producer`. The ~0.2 s that remains is the
per-candidate fact probes (18.5k candidates in stream 1); it holds nothing.

### Boot write holders: a finished scan's findings (measured 2026-11-01)

`finish_scan_with_findings` held the write lock 366 ms at boot: a scan files
up to 7,000 findings (the live database's largest), and each one ran a
prepared-every-time insert, then `replace_source_tx` for its file edge (two
deletes for a source that cannot have edges yet, then an insert). Measured
(debug build, in memory, 5,000 findings): 200 ms. Findings now go through a
cached statement and `page_ref_store::insert_new_sources_tx` (one cached
insert, no deletes): 84 ms. `every_finding_of_a_finished_scan_gets_its_file_edge`
pins that every finding keeps its edge.

### Boot write holders: a snapshot take (measured 2026-11-01)

`record_take` held the write lock 426 ms. Inside it, a take read the whole
tree at its parent (latest row per path over all of the stream's snapshot
rows: 46k rows for 2.7k files on this database, 134 ms for stream 1, 72 ms
for stream 2; bundled SQLite) to hash the tree it was making. The tree is now
read before the lock (`SqliteSnapshotStore::record_take`), and the write
lays the take's rows over it (`overlay`); when another take landed between
the read and the write the tree is read under the lock as before. The
remaining hold is the row inserts, the hash of the tree and the event.
`a_takes_stored_hash_is_the_hash_of_the_tree_it_stored` pins that the stored
hash equals the hash of the tree as stored for the overlaid and the stale
case, with a change, an addition and a deletion.

### Boot write holders: the per-asset start-up writes (2026-11-01)

The 32 `assets.rs:207` writes were `track`: one write transaction per asset
at every start to make triggers that already exist (`CREATE TRIGGER IF NOT
EXISTS`), each queued behind the other holders (held 0 ms, waited up to
0.8 s). `track` now reads which tables lack their triggers
(`table_generations::untracked_tx`) and writes only for those: none after the
first start. The 32 `assets.rs:821` writes are the `asset_state` row an
asset records after a recompute; an asset whose inputs, definition and build
are unchanged skips both the recompute and that write (`is_current`), and the
daemon log's 32 were a rebuilt binary (`build_identity` is part of what an
asset is built from), so a dev restart after a rebuild recomputes them all.
Those writes are left as they are: each follows work that changed something.
Pinned by `only_a_table_without_its_triggers_is_untracked`. Live effect is not
measured: the acceptance is five minutes of log after `run:dev` with at most
one hold over 300 ms and no wait over 1 s.

### A search re-index reads on from its watermark (2026-11-01)

Every change to a table behind a searchable kind re-read that kind's whole
feed (every row's version, or for an unversioned kind every row's text) to
find the few rows that differ. `KindSearchIndex::recompute(full)` now reads,
for a core kind whose view has a `seq` column, only the rows past the highest
`seq` it indexed when the inputs saw inserts only; `full` (any UPDATE or
DELETE, the first build, a deploy or vocabulary change) and a fresh process
read whole. `search_decision` and `search_turn` declare `seq` (their row
id); the others cannot say that no insert changes or removes an existing row
(see `extensions-events.md`). Pinned by
`an_insert_only_recompute_reads_only_the_new_rows`: three decisions, a whole
read of 3; one more, a read of 1; nothing new, a read of 0; a whole read of
4 leaves the same entries. The saving is bounded by the kinds that qualify
and by `full` (a turn's end is an update): this is not measured on the live
database.

### Opening a folder on the Files page (measured 2026-11-01)

Expanding a folder runs the Directory page once for that folder (a lazy
branch), and every open run re-runs on a change. The daemon log showed
`slow page re-run … page=oxplow-foundation/files` at 560-900 ms, even
with `loaders=` empty. That list is the loaders whose *rows changed*, not
the ones that ran: a re-run runs every loader its changed dataset
reached and then finds equal rows. Measured on this repo (16 refs, idle
machine):

| part of one folder listing | before | after |
|---|---|---|
| `row` loader: `v_branch_divergence d WHERE d.base = v.scope AND d.head = 'HEAD'` | 444 ms (a `compute_divergence`, merge simulation included, for each of 16 refs) | 0 (read only for a branch comparison, one pair) |
| three libgit2 status scans (conflicted, staged, statuses) | 3 x 30 ms | 30 ms (`oxplow_git::worktree_status`) |
| filter rebuilt by walking every folder for `.gitignore`s | 35-100 ms | about 1 ms (`WorkspaceFilter::for_folder`: root, the way to the folder, the folder) |

Causes: `d.base = v.scope` compares to a column, so no pair is named and
the dataset ranges over all branches (`LiveSources::branch_pairs`); the
dataset is read whatever the `CASE` later does, so it cost the same with
Show at "all files". The fix is the `range` loader, which names the one
pair (`d.base = :eff_scope`). `v_file` itself was already one `read_dir`
(pinned by a test on the page's own SQL). Rule: a live dataset is read
when the query mentions it; condition it with a param equality, not a
join or a `CASE`.

## Daemon transport: what a loopback HTTP hop costs (tsk255)

Measured for the daemon-backed shell epic ([[tsk254]]), from **inside the real
WKWebView** with embedded assets (the packaged path), against a local
`oxplow-daemon`. The comparison that matters is Tauri IPC vs daemon HTTP — *not*
"in-process vs network" — because today's local path is already JSON over
Tauri's IPC bridge, not a function call.

| call | Tauri IPC | daemon HTTP |
|---|---|---|
| `ping` (no work) | p50 ~0 ms, p95 1 ms | p50 4 ms, p95 5 ms |
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

`extensions::load_extensions(root)` parses every bundled page file (about
forty embedded YAML documents) plus the project's on each call, and it ran
on every advisory check (each agent tool call and prompt), every
`list_extensions` (IPC) / `oxplow_read` of a page / `oxplow_run_page`, the schema listing and metric
catalog seed. Measured on the dev machine (release-less test build, 20
calls averaged):

| call | per call |
|---|---|
| `load_extensions`, bundled only | ~3.3 ms |
| `load_extensions`, bundled + one project extension with 10 pages | ~3.1 ms |
| `ExtensionCatalog::get`, same root, cache hit | ~62 µs |

The daemon's catalog (`ExtensionCatalog::for_project`) loads the live set
once per generation, from the deployments in the store; it reads no source
and keeps no per-root cache or fingerprint. Source loads
(`load_extensions`, used by `oxplow_validate_extension` and the CLI) parse on
every call. A deploy's hash always reads the bytes it
deploys. A cache entry owns a `CatalogSnapshot`: declarations and the
generation hash. No new performance measurement was run.

## Metric visibility: ancestry in bulk (2026-10-06)

Every boot pegged one thread for 8–12 minutes in
`metric_visibility::resolve`: it asked git about each (absorbing commit,
base commit) pair with its own `graph_descendant_of` walk, and this
repo's captures have ~860 distinct anchors on each side — hundreds of
thousands of walks, under the oracle mutex every other metric reader
waits on. `RevisionGraph::ancestry` now answers the whole cross product
from one parents-first revwalk that ORs each commit's parents' bitsets
of anchors (`vcs/git.rs`), and `GraphOracle` asks the graph once per
resolve. Its answer is the compact `Ancestry` (an index per commit and a
bitset of its ancestors), which the oracle keeps as ONE shared copy —
rebuilt only when a resolve brings an anchor it hasn't seen — and every
`Visibility` holds by `Arc`. As a map of sha pairs it was ~1.16 M entries
(~200 MB) copied into each read. Measured on this repo, debug build:
1,079 anchors over 2,158 commits in 50 ms, 143 KB of bitsets (the pair
map took 1.2 s, mostly allocating it). Don't go back to per-pair
questions or pair maps; the cross product is not "tiny by construction",
and at 50 ms it isn't worth persisting.

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

## Effort deltas recompute only what moved (2026-10-06)

Token counts land as captures every ~18 s, and each one, through the cube,
recomputed every open effort's delta for every spec. `CollectionService`
now memoizes each (effort, spec) delta on what it's computed from: the
spec, the effort's span, start/end snapshots, stream and claimed files,
its own captures, and the capture token of its source measure's
producers. A token capture moves only the token specs' inputs; the rest
reuse their row. A formula spec (no source measure) always recomputes.
Together with the per-measure memo (producers, slice keys,
representatives; `.context/metrics.md`) and the cube skipping measures
with nothing new, a burst of token writes costs a few indexed lookups per
spec rather than history scans.

## Assets skip their boot rebuild (2026-10-06)

Every asset rebuilt at boot because which tables changed was known only
in memory. Tables an asset reads or writes now carry triggers bumping a
per-table generation (`table_generation`), and an asset whose recorded
`built_from` still matches skips its first build. The trigger's cost,
measured with the sqlite3 CLI: 200k single-statement inserts into a bare
two-column table took 86 ms with it against 41 ms without — about 0.2 µs
a row, proportionally less on real tables with indexes.

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
core dataset compile — **~330 ms per test**, in its own process under
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
(`signal_group`).

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

## Core's vocabulary is built once per process (2026-10-07)

Building a test's `Services` had crept back to ~0.3 s, and `oxplow-app`'s
median test to 0.37 s. A `sample` of repeated builds put most of it in
`VocabularyHandle::core()`: every store that validates events took its
own handle, and each one compiled every core event type's JSON schema
(`EventSchemaRegistry::core()`) again — four-plus times per `Services`,
more than loading the bundled extension. Core's vocabulary can't differ
within a process, so `VocabularyHandle::core()` now shares one
`LazyLock<Arc<Vocabulary>>` (a swap still replaces only that handle's).
`oxplow-app`'s suite went from 1,265 to 622 test-seconds (91 s → 46 s
wall, median test 0.37 s → 0.13 s) on the same loaded machine; daemon
boot gains the same.

What's left in a fixture, measured: the bundled extension's load
(`ExtensionCatalog::get`, about twice one vocabulary build) — parsed once
per `Services`, and nextest runs each test in its own process, so a
process-wide cache wouldn't help tests. Optimizing bundled SQLite
(`opt-level = 3` for `libsqlite3-sys`) changed nothing measurable
(`oxplow-db`: 9.2 s vs 9.0 s); a test process's own start is ~15 ms.

## Fact dictionaries (V38, 2026-10-08)

Facts store their subject, path and dims as ids into `fact_subject` /
`fact_path` / `fact_dims` (see [metrics.md](./metrics.md)). Measured with
`cube_equivalence` on copies of the live database (2.58 GB, 6.3 M facts),
release build, against `main` on an identical copy:

| | before | after |
|---|---|---|
| fact table + its indexes | 1.83 GB | 0.52 GB |
| whole database, vacuumed | 2.58 GB | 1.16 GB |
| fact-served reads (72 specs) | 24.8 s | 24.8 s |
| cube reads | 4.9 s | 4.9 s |
| full cube build from empty | 69 s | 77 s |

The migration itself took ~100 s on that copy. The first version resolved
the text with per-row scalar subqueries in the SELECT list: reads went
**+26%** (25.5 s → 32.2 s), because a read returns the same subjects
across many rows. Resolving ids once per read in Rust (`FactText`)
brought reads back to par; don't reintroduce per-row lookups.

## Change-only complete scans (V39, 2026-10-08)

A complete-scope capture stores only what changed since its producer's
previous scan and holds the rest (`fact_chain`, see
[metrics.md](./metrics.md)). The live history rewritten into chains,
measured with `cube_equivalence` against `main` on identical copies of
the live database (1.30 GB, 6.3 M facts), two runs each:

| | main | chains |
|---|---|---|
| fact rows | 6.27 M | 3.29 M |
| fact table + its indexes | 529 MB | ~280 MB |
| whole database, vacuumed | 1.30 GB | 0.91 GB |
| fact-served reads (72 specs) | 22.7 / 23.9 s | 24.2 / 21.3 s |
| `oxplow.duplicate_lines` fact read | 5.6 s | 5.0 s |
| full cube build from empty | 67 / 71 s | 78 / 76 s |

`v_fact` returned every row of every complete-scope measure identically
before and after the rewrite (order-free digest). The full build pays a
few seconds for the chained captures' per-fact join; incremental builds
fold only new captures.

Two planner traps on the chain join, both pinned in `fact_chain.rs` and
`fact.sql`: without CROSS JOINs the planner (no stats for an empty
`fact_chain`) drove from every fact and scanned the stream's later
captures for each — a `v_fact` digest that takes 11 s ran past ten
minutes; and with `f.capture_id BETWEEN ch.from_capture_id AND
ch.capture_id` it used only the lower bound, scanning every later fact of
the measure. Spell the bounds out as `>=` / `<=`.

## Per-path holds (V43, 2026-10-08)

A code gauge's scan holds each rescanned file's unchanged facts, a moved
function a line move (`fact_path_hold`, `fact_line`, see
[metrics.md](./metrics.md)). The live history rewritten into holds,
measured against `main` on identical copies of the live database:

| | main | holds |
|---|---|---|
| fact rows | 3.47 M | 1.17 M |
| per-path fact rows | 2.64 M | 0.34 M (+1.21 M `fact_line`, +50 k holds) |
| fact tables + their indexes | 301 MB | 132 MB |
| whole database, vacuumed | 1.09 GB | 0.78 GB |
| fact-served reads (72 specs) | 24.0 s | 24.7 s |
| full cube build from empty | 79.7 s | 79.5 s |

`v_fact`'s per-path rows (lines included), `v_tree_fact` and `v_function`
read identically before and after the rewrite, and all 72 series are
identical.

**Name the index on a held read.** Inside the whole read (three UNION ALL
branches and an ORDER BY) the planner read a hold's facts through
`idx_fact_measure_capture` — every fact of the measure in the hold's
capture range — instead of `idx_fact_measure_path`, which seeks the one
file: the per-path history read went 2 s → 21 s and the fact-served specs
24 s → 174 s, while the same join counted alone chose right. The chain
and hold joins name their index (`INDEXED BY`), in `fact_chain.rs` and
both datasets.

## Related

- [metrics.md](./metrics.md) — the metric substrate itself: the cube, its two
  counters (`epoch` fences writers, `version` invalidates the read cache), the
  event-scoping and debounce stack, and the retention/compaction knobs.
- [data-model.md](./data-model.md) — the DB pool and `spawn_blocking` cap.

## Change events reach the webview in bursts

A burst of commits is one re-read in the UI: every reader of change events
goes through `coalesce` (`apps/desktop/src/viz/rerun.ts`, 100 ms after
the last event) — `useRerunOnChange`, the wiki title cache
(`wikiTitleCache.ts`, which also re-reads once after an in-flight read
that missed a change) and `useChange`. A new reader of events adopts it
rather than re-reading per event. Writers help from their side by
committing a unit of work once (a duplicate scan stores its findings in
one transaction). **The boot page-ref repair** (`page_ref_backfill.rs`,
run once per projection-code hash and schema version — it used to be
once per build, ~59 s of a thread at every restart after a dev rebuild)
restates every work item, effort, note and finding slice: it writes them in
batches of 1000 through `SqlitePageRefStore::replace_sources`, one
transaction each. Written one commit per row, it once sent ~250k change
events at boot (a quarter of a million piled-up findings): the events
socket lagged, the UI re-read for a minute and terminal input timed out.

## What running scripts in workers costs (measured 2026-10-09)

Every Starlark and jaq run is its own `oxplow-script-worker` process
([collection.md](./collection.md) "Containing scripts"). Measured with
`crates/oxplow-script/tests/worker_cost.rs` (ignored by default; `bun run
test:fast -p oxplow-script --run-ignored only --no-capture worker_cost`),
debug build, Apple silicon:

| What | Cost |
|---|---|
| A cold trivial run (spawn, run, exit), median of 20 | 34 ms wall, 2 ms worker CPU |
| The whole-workspace lcov parse (6 MB, 100k lines), bundled parser | 2.3 s |
| Shipping a 120-file / 663 KB tree to a fact collector | ~8 ms |
| Each built-in fact metric over that tree | 2–11 MB peak; 17 ms–1.3 s CPU |
| `oxplow.duplicate_lines` over that (duplicate-dense) tree | 527 MB peak, 11 s CPU |
| `oxplow.duplicate_lines` over this repo's 1159 source files (14 MB) | 67 MB peak, 17 s CPU, 23,050 facts |

**Decided: spawn per run, no warm workers.** The rule set beforehand
(script-containment plan): reuse workers only if a cold start plus tree
shipping costs over 100 ms, or over 10% of a realistic collector's run.
It's ~42 ms, and a fact collector over a real tree runs for hundreds of
milliseconds to seconds. A command script pays the ~34 ms start inside
its bus transaction; if that lock time ever shows, a warm worker for
command scripts is the lever (a worker is still killed on any overrun).

**The 512 MB default cap holds for real trees**, with one known way to
reach it: `duplicate_blocks` reports every *pair* of copies, so a block
with k copies yields k(k−1)/2 pairs and facts for both sides of each.
The generated corpus (shared blocks spliced into a fifth of its files)
needs 527 MB; this repo needs 67. A heavily copy-pasted repo can hit the
cap, and `duplicate_lines` then fails with "ran out of memory" rather
than taking the app's memory with it.

## Serving a tree on demand isn't worth it (measured 2026-10-14)

A collector that declares `tree.read` gets the snapshot's whole file map,
read once on the host (`MetricsService::build_full_file_map`, cached for
the last snapshot) and shipped to each worker as one JSON line. Whether
to serve it lazily instead (a worker asking for files as it needs them)
was measured first: `worker_cost.rs` `tree_cost` (ignored; `bun run
test:fast -p oxplow-script [--cargo-profile release] --run-ignored only
--no-capture -E 'test(/tree_cost/)'`) runs the 16 oxplow-foundation
code-metric scripts over this repo's text files, once and six times over:

| Tree | Build | Host read | Ship + parse, 16 runs | The metrics' own work |
|---|---|---|---|---|
| repo, 2,067 files / 20 MB | debug | 0.3 s | 8.2 s | 157 s wall, 148 s CPU |
| repo, 2,067 files / 20 MB | release | 0.4 s | 1.5 s | 75 s wall, 41 s CPU |
| repo ×6, 12,402 files / 122 MB | release | 0.6 s | 9.5 s | 210 s wall, 168 s CPU |

(The ×6 tree in a debug build runs past the 120 s default timeout per
script.) Shipping and parsing the tree is 2–5% of a sweep and reading it
on the host under 1%, so neither a lazy protocol nor a bigger host cache
pays for itself. **Decided: the tree stays shipped whole.** The cost is
the scripts: each of the 16 walks every file it globs and parses it
again (`ast_query`, `code_metrics`, `markers` per file per script), so
a file was parsed by tree-sitter once per script that reads its
language.

**The 75 s wall vs 41 s CPU gap was contention, not the pipeline.**
`cpu_ms` is the worker's own `getrusage`; wall time also counts every
moment the worker (or the host thread feeding it) waited for a core. The
75 s sweep was measured while other builds and test runs shared the
machine. Re-measured on an idle machine the same sweep is 28.6 s wall for
27.7 s of worker CPU (the other 3%: spawn, shipping the tree, the
facts); with 28 busy loops on the 14 cores the 120-file sweep goes from
3.1 s CPU / 3.6 s wall to 3.3 s CPU / 10.3 s wall. So output
serialization and fact transfer, the earlier suspects, cost nothing
visible; ranking the scripts' worker CPU (`tree_cost` prints the top
eight) puts the time in tree-sitter parsing: the four `code_metrics`
scripts at about 2 s each over this repo, the Rust and TypeScript
`ast_query` scripts at 1.4–4.6 s, `duplicate_lines` at 3.4–3.9 s.
(Attributed by script CPU, not with samply: the parse is inside one
builtin call per file, so a per-script ranking already names it.)

**Parse results are shared across a sweep.** Workers are one process per
run, so nothing in them can outlive a script; the host does
(`ScriptWorkers`' parse cache, `workers.rs`). `code_metrics`, `markers`
and `ast_query` ask the host for the JSON they would return under a key
of (builtin, language, whatever else the answer depends on, SHA-256 of the
content) — `CallKind::Parse` `get`, and on a miss compute and `put`. An
edited file is a new key; an unchanged one is read, not parsed, by every
later script, sweep and snapshot while the host lives. The cache holds up
to 256 MB of JSON text, dropping the oldest first. In-process runs have no
cache and compute as before. Pinned by
`a_file_is_parsed_once_across_a_sweeps_scripts` (hit and miss counts,
identical facts to a cold worker, an edit re-parses only its file).

| `tree_cost`, release, idle machine | Before | After |
|---|---|---|
| fixture, 120 files | 3,087 ms CPU | 1,401 ms CPU |
| repo, 2,068 files / 20 MB | 28.6 s wall, 27.7 s CPU | 21.6 s wall, 20.7 s CPU |
| repo ×6, 12,408 files / 123 MB | 192.8 s wall, 166.8 s CPU | 43.7 s wall, 40.6 s CPU |

What is left: the first script to read a language pays its parse, and
`ast_query` scripts with different queries over one file (the three Rust
and four TypeScript ones) each parse it again, because the shared
thing is a *result*, which depends on the query, not the syntax tree,
which can't cross a process. `code_metrics` keys on the path too (its ids
and refs contain it), so the ×6 tree's copies still parse once per path
there; `markers` and `ast_query` hit across copies. The next lever is one
parse serving several queries (the host running the queries, or a script
naming all its queries in one call); `duplicate_lines` is its own cost.
`TREE_COST_SKIP_X6=1` skips the ×6 tree for a quick run.

## Duplicate detection on clone-dense trees (measured 2026-10-09)

The detector once paired every member of a clone family with every other,
so a tree dense with duplication held n(n-1)/2 pairs per family, and the
`duplicate_lines` collector turned each into a JSON and a Starlark value
(about 512 MB on a generated dense corpus, against about 67 MB on the real
repo). Each member is now linked to one peer (`.context/code-quality.md`).

Measured on an Apple M4 Max with `crates/oxplow-code-dup/examples/dense_rss.rs`:
`detect_duplicates` over D directories of F files, every file holding the
same 13-line function, then a `serde_json` object per block as the
`duplicate_blocks` builtin builds them. Peak RSS is `/usr/bin/time -l`'s
"maximum resident set size" of a release build, before (the parent
commit, built in a scratch worktree) and after:

| Corpus | Blocks before → after | Peak RSS before → after | Time before → after |
|---|---|---|---|
| 1 × 60 files | 1,770 → 59 | 5.3 MB → 3.4 MB | 7 ms → 8 ms |
| 12 × 60 files | 258,840 → 719 | 314 MB → 5.3 MB | 297 ms → 72 ms |
| 24 × 60 files | 1,036,080 → 1,439 | 1,244 MB → 6.9 MB | 1,542 ms → 131 ms |

The worker-cost sweep's 120-file tree above is that shape: with the fix its
whole 16-metric sweep takes about 7.2 s of worker CPU (7.9 s wall), where
`oxplow.duplicate_lines` alone took 11 s and 527 MB before. (`worker_cost`
prints the sweep's total, not each metric's peak, so the per-metric figure
isn't re-measured here.)

Before, memory grew with the square of the family (the number of blocks
quadrupled when the files doubled); after, with the file count. The same
shape on the real repo's own duplication is unchanged in kind: its families
are small, so the saving there is small.

```sh
cargo build --release -p oxplow-code-dup --example dense_rss
/usr/bin/time -l target/release/examples/dense_rss 12 60
```

## Duplicate detection cache (measured 2026-10-09)

The change-scoped duplicate scan keeps each file's eligible subtrees
("records", 32 bytes each) in memory between scans, keyed by content id
(`.context/code-quality.md`), so an unchanged file is neither read nor
parsed again. The budget, `CACHE_BUDGET_BYTES` in
`crates/oxplow-code-dup/src/cache.rs`, is 32 MB (about a million records)
and is a constant, not a setting, until someone needs one.

How it was sized, with `crates/oxplow-code-dup/examples/cache_size.rs`
(release build, Apple M4 Max, `DupOptions::default()`):

| Tree | Source files | Records | Cache | Cold scan |
|---|---|---|---|---|
| this repo's `crates/` | 548 | 85,655 (156 per file) | 2.6 MB | 2.3 s |
| this repo's `apps/desktop/src` | 589 | 26,858 (46 per file) | 0.8 MB | 0.9 s |

Together about 3.4 MB for 1,137 files, so 32 MB holds roughly ten times
that, around 11,000 source files of this shape: room for a large
monorepo, and small enough to forget about. A repo that doesn't fit isn't
wrong, only slower: the least recently used files are evicted whole and
re-parsed when next scanned. A single file with more than the whole
budget's records is never kept.

A warm scan parses nothing and reads nothing: its time is the pairing
over the records. Each scan's log line (`duplicate scan`) carries
`cache_hits`, `cache_misses` and `elapsed_ms`, so a cold scan after a
restart is visible.

## Reading a big lcov (measured 2026-10-09)

The coverage report collector read a 12 MB `lcov.info` (read after every
`test:collect`) through the bundled jq program: the host split the report
into records, serialized them to JSON, ran the program in a script worker,
and deserialized its answer. Where the time went, on a generated 14.2 MB /
240,000-line report of 603 files (`tests/lcov_cost.rs`, Apple M4 Max; the
debug build is what developers run, and the interpreter is ~6x slower
there):

| Stage | Release | Debug |
|---|---|---|
| Split into records (native) | 17 ms | 110 ms |
| Serialize them to JSON (13.9 MB) | 7 ms | 224 ms |
| The jq program, including parsing that JSON | 626 ms (26 ms of it the input) | 4,332 ms (386 ms) |
| Coerce its answer into typed coverage | 5 ms | 24 ms |
| Whole `Collector::run` (worker included) | 733 ms | 5,484 ms |

So the cost was the program: about 100,000 `DA` entries, each split,
`tonumber?`-ed and built into an object by the interpreter. Parsing and
coercion were noise, and nothing was quadratic any more (the earlier
`reduce` fix held).

The fix is a native parser for the bundled `lcov` collector
(`helpers::lcov_coverage`): one linear pass from the text to the typed
`CoverageReport`, in process, with no JSON in between and no worker to
spawn. The jq program stays as the reference, and a test holds the two to
the same reading on a generated report and the edge cases (a repeated
`SF`, a trailing record, junk and short `DA`, the first of each summary
line).

| | Release | Debug |
|---|---|---|
| Before (jq in a worker) | 733 ms | 5,484 ms |
| After (native) | 10 ms | 64 ms |

One behaviour differs at the margins: a `DA` whose line isn't a whole
number (`2.5`, `-1`) is skipped, where the program passed it on and the
typed coercion then failed the whole report. The other bundled parsers
(junit, cobertura, jacoco, clippy, eslint) read XML or JSON and were not
measured as costly.

## Bulk fact reads leave `detail` out (measured 2026-10-09)

A `fact`'s `detail` is the one large text it can carry (an analysis finding's
message). Every history load (`facts_for_measure`, the stream and live
reads, the held-facts read) selected it and allocated a `String` for each
row that had one, though the paths that fold, filter or bucket facts never
look at it. Bulk reads now select `NULL` in its place
(`FACT_ROW_COLS_SLIM`, the same column list as `FACT_ROW_COLS` from one
macro, so the mapper's positions don't move), and the findings read, the
only reader that shows a message, asks for those of the facts it returns
by id (`SqliteFactStore::fact_details`). The chain write path keeps the
full list: it compares a held fact with a new one by `detail`.

Measured on 100,000 facts of one measure, each carrying a 230-byte message
(the worst case; most facts carry none and read the same before and
after), release build, median of five `facts_for_measure` reads
(`fact_store::tests::fact_read_cost`, ignored):

| | Median read |
|---|---|
| `detail` selected | 144 ms |
| Left out (`NULL`) | 96 ms |

A third of the read, in the case where it can matter; nothing where facts
have no message.

## Threshold counts from the cube (measured 2026-10-09)

The three specs the cube declined (`oxplow.long_functions` and
`oxplow.high_complexity_fns` `min_value` counts, `oxplow.coverage.untested_files`
a `max_value` count) folded per-path state from raw facts on the first read of
each generation — 698, 685 and 544 ms on the 6 M-fact copy. A cube bucket now
also records whether its facts pass each value bound the measure's specs use,
so these are sums over the passing buckets (`.context/metrics.md` "Value bounds
are part of the grain").

On a synthetic per-subject measure (600 captures of 3,000 functions, a tenth
re-measured each capture, release; `metric_cube::tests::threshold_read_cost`,
ignored), one `min_value` count:

| | ms |
|---|---|
| Read from the facts | 227 |
| Cube build (once; incremental after) | 994 |
| Read from the cube | 1 |

The build is the one-time fold every cube-served measure pays at write time,
and a changed bound set re-folds that measure once. Equality with the facts is
held by `threshold_specs_are_cube_served_and_equal_the_facts` (counts and a
sum, min and max bounds, a value on the bound, restated subjects crossing it
both ways) and `a_per_path_max_value_count_is_cube_served_and_equals_the_facts`.

**On real data** (`cube_equivalence` on a `VACUUM INTO` copy of the live
database, 1.37 M facts, 162,275 captures folded in 98.7 s; the live file was
never opened by the harness): **all 72 series identical to the fact oracle**,
72 specs in 27,778 ms from facts → 1,823 ms with the cube, and the three
specs are no longer among the ones the cube declines (5 remain, all
unrelated: `oxplow.coverage.branch_pct`, `repo.eval_usage`,
`repo.focused_tests`, `repo.hardcoded_secrets`, `work.open_tasks`):

| spec | From facts | From the cube |
|---|---|---|
| `oxplow.long_functions` | 1,878.9 ms | 7.8 ms |
| `oxplow.high_complexity_fns` | 1,818.7 ms | 7.9 ms |
| `oxplow.coverage.untested_files` | 579.5 ms | 29.0 ms |

## `v_agent_session_status` starts from the open sessions

It used to walk every `agent.status.changed` ever logged and, for each, the
session's events: over 5 s on a real log (2 GB), past `query_sql`'s limit, and
it held the one background database slot, so everything queued behind it.
Starting from the open sessions and taking each one's newest event takes 18 ms
on the same data. A query plan test holds it (`agent_stores`). The
`slow page loader` log line names a loader of 50 ms or more, and a page's
`spec` run through `oxplow_run_page` times a query in place.
