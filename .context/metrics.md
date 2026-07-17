# Metrics — the unified metric substrate

What this doc covers: oxplow's **metric substrate** (epic tsk213) — one durable,
typed model for any deterministically-computable number tracked over time, the
successor to `effort_observation` and (eventually) `code_quality_*`. Coverage,
tests, clippy findings, token usage — all become *metric definitions* over the
same tables, queryable by the agent (MCP) and the renderer (IPC), and surfaced
on the Metrics page.

Status: substrate + read surface are **live** and are now the **sole** store for
coverage/test/analysis facts — the legacy `effort_observation` table was
**dropped** (tsk215). The effort-review panel reconstructs its rows from the
substrate (`CollectionService::effort_observations_from_metrics`); the
`EffortObservation` type survives only as the read/IPC shape.

> **✅ Cutover complete (epic tsk12).** The V38
> `metric_definition`/`metric_run`/`metric_sample`/`metric_finding` cluster and
> `metric_store.rs` are **gone** (reads flipped in T-C2/T-C3/T-D/T-E1; writes
> dropped in T-E2; tables dropped in V49, T-E3/tsk50). The fact substrate is
> the sole metric store. Sections below that describe V38 mechanics are
> historical context for why the model looks the way it does.

## The fact substrate (epic tsk12 — the inversion, in flight)

**The defect being fixed.** V38 has facts and metrics *inverted*. `metric_sample`
is labelled "the BI fact grain" but holds *pre-aggregated, per-metric* values
(a count-over-threshold, a `tree:.` repo sum) — aggregation baked in by the
collector. `metric_finding` holds the *atomic, re-aggregatable* facts
(function→complexity, test-case→pass/fail, lint hit) — but ephemerally, as
CASCADE-with-run drill-in, not as a durable queryable series. So a new metric
over the same reality needs a *new gauge that re-walks the code*: aggregation is
welded to collection.

**The fix (headless-BI / semantic-layer model).** Invert the source of truth:
- **facts** = durable atomic measurements (the real grain) — the source of truth,
- **measures / dimensions** = the conformed catalogs facts are typed by,
- **metrics** = *aggregation/formula definitions computed over facts at read time*
  — never a second pile of stored rows. The materialized series is a rebuildable
  cache, not the truth.

### Schema — `crates/oxplow-db/migrations/V43__metric_facts.sql`, store `crates/oxplow-db/src/fact_store.rs` (`SqliteFactStore`)

> ### ⚠️ Two axes, not one: `temporal_semantics` × `capture_scope` (V54, tsk41)
>
> **`temporal_semantics`** says how values combine OVER TIME.
> **`capture_scope`** says how much of the population ONE capture speaks for.
> They are orthogonal, and conflating them was a real bug.
>
> - `capture_scope = complete` (default) — every capture restates the whole
>   population (a coverage report, a clippy run, the whole-tree duplication scan).
>   The temporal fold applies directly.
> - `capture_scope = per-path` — a capture restates **only the paths in its
>   snapshot**. This is what a **tree gauge** does: after the initial full index,
>   every snapshot is a per-commit **delta** (5–19 files).
> - `capture_scope = per-subject` (V55, tsk43) — a capture restates **only the
>   subjects it emitted facts for**. `oxplow.test_case`: a **partial** run
>   (`bun test src/foo.test.ts`) holds just those cases, so read as `complete` the
>   metric would report *"the repo has 4 tests"* and lose every failure elsewhere.
>   Folded to the **latest fact per `(producer, subject_ref)`**, a partial run updates
>   only the tests it ran and the rest keep their last-known status
>   (`latest_subject_facts`). Trade-off: a **deleted/renamed test lingers** — a run
>   can't say "this subject is gone" the way a `storage='deleted'` file row can.
>
> **The bug this fixes.** The tree measures were `semi-additive` ("take the last
> capture") — correct only if captures are complete. Against delta captures that
> reads as *"the repo is only the 8 files I just touched"*:
> `oxplow.rust.unsafe_blocks` reported **0** while the repo had **15**.
>
> **The fold.** A `per-path` measure's value = for each `(producer, path)`, the
> facts from the **latest capture of that producer that scanned that path**
> (`SqliteFactStore::latest_tree_facts`; per-capture trend via
> `metric_engine::tree_state_series`). What a capture "scanned" is its
> **`metric_capture.scan_kind`** (V58, tsk71):
> - **`delta`** (default) — the capture's own snapshot `file_snapshot` rows: the
>   ordinary incremental rescan of just-changed files.
> - **`full`** — the **reconstructed tree as-of its snapshot** (`tree_at`'s
>   window: latest row per path ≤ the anchor, tombstones included so deletions
>   still drop out). This is what a **baseline** records — it restates the whole
>   tree while anchored to an ordinary delta snapshot, so no full-tree snapshot
>   is ever fabricated.
> - **`asserted`** — exactly the paths it emitted facts for (agent
>   `record_metric`, synthetic writes). Its snapshot is **provenance only**,
>   never a scanned set; the insert coerces any snapshot-less capture to
>   `asserted` (delta/full require an anchor).
>
> The scanned set never comes **from the facts a scan emitted** — which is why
> the whole thing needs *no* write-side convention:
> - a file whose count drops to **0** emits no fact (`if c > 0:`), but its path is
>   in the new snapshot ⇒ the new capture supersedes the stale value. **No
>   zero-emission convention; the bundled gauge scripts are untouched.**
> - a **deleted** file's latest row is a `storage='deleted'` tombstone ⇒ dropped.
>   **No tombstone facts.**
> - **symbol**-grained facts and **many-facts-per-path** (TODO markers) are
>   superseded *wholesale per file*, so a removed function/marker disappears.
> - partitioning by **producer** matters: the 10 idiom gauges share
>   `oxplow.ast_hit` (sliced by `rule`), so without it a later gauge's capture would
>   supersede an earlier gauge's facts for the same path.
> - partitioning by **stream** matters for the same reason (tsk98): a stream is a
>   **worktree**, and the fold reconstructs *one worktree's tree*. Two worktrees
>   running the same gauge share `(producer, path)` keys, so a stream-blind state
>   lets one worktree's capture evict the other's paths — yielding a point that is
>   whichever worktree wrote last, per path, and belongs to neither. The state is
>   therefore keyed **`stream → (producer, path)`**, matching the scoping the fact
>   fetch (tsk75) and the rollup (tsk46) already apply.
>
> **An unscoped (`stream = None`) read is a UNION, not a merge.** It replays every
> stream's captures into one timeline, but each point is still exactly one
> worktree's state, and carries its own `stream_id`/`branch`. Streams are a
> **dimension you can group by, never a partition that hides rows** — so
> cross-worktree comparison works, while no point ever blends two worktrees. There
> is deliberately **no** "merged cross-stream tree": it would describe no worktree.
>
> `zero_fill` is **suppressed** for `per-path`: an empty delta capture restated no
> paths, so it means "nothing changed", not "the repo is zero".
>
> **Baseline (tsk71 — no fabricated snapshot).** A gauge's repo-wide total needs it
> to have restated the whole tree once — that's a **`scan_kind='full'` capture over
> the reconstructed tree of an ordinary snapshot** (corpus via
> `SqliteSnapshotStore::list_tree_files_at` → `build_full_file_map`), NOT a
> fabricated full-tree snapshot. The old `enqueue_full_tree` (mark every path dirty,
> capture a snapshot listing the whole tree) is **gone**: that snapshot polluted
> effort file-attribution — an edit from effort A that hadn't been snapshotted yet
> first landed in a snapshot inside whatever effort window was open when a rebuild
> ran, producing false "changed but not claimed" EFFORT REVIEW flags.
>
> `gauges_needing_baseline` is the **pending-baseline queue**: the on-snapshot sweep
> (`run_snapshot_gauges`) partitions gauges every time a snapshot lands — already-
> baselined gauges run `delta` over the snapshot's own rows; queued ones run `full`
> over the reconstruction. So a newly added/edited gauge baselines on the next
> ordinary snapshot automatically. **`Services::rebuild_metric_baseline(force)`** is
> the on-demand entry point — it waits for the startup sweep, drains genuinely
> pending edits into a NORMAL snapshot (real authored work, correctly attributed;
> none dirty ⇒ **no snapshot is created**, it anchors to the latest existing one),
> then runs the sweep. **Boot, the `rebuild_metrics` MCP tool, and the end-to-end
> tests all call it** (tsk50) — see
> `rebuild_does_not_fabricate_a_snapshot_on_a_clean_tree`. **Not** needed on a
> branch switch — checkout rewrites the differing files, the watcher marks them
> dirty, and the delta rescans exactly those paths.
>
> The sweep is **idempotent per (snapshot, gauge, fingerprint, scan_kind)**
> (`gauge_done_for_snapshot` — kind-scoped so a delta capture can't satisfy a
> pending full baseline over the same snapshot): a repeat rebuild or the event loop
> reacting to the same snapshot won't double-scan the tree (the manual `run_metric`
> path bypasses it — an explicit "run now" always runs).
>
> **Dominated-capture GC (tsk75).** A fresh baseline makes every effort-less
> `delta`/`full` tree capture strictly OLDER than it dead weight (the dominance
> argument: the baseline restates every path, newer). Their facts had grown to
> ~69% of a 778k-row fact table (~178k rows EACH on the per-function measures)
> and every full-history read paid for them — the effort-panel refetch loop over
> that history is what saturated the daemon. `prune_dominated_tree_captures`
> deletes them (facts CASCADE) after each clean full sweep and once per boot.
> Deliberately narrow: effort-stamped captures survive (attribution history),
> captures carrying any non-per-path-measure fact survive, producers with no
> baseline survive, asserted/failed captures survive. Accepted trade-off: a
> per-path measure's TREND loses pre-baseline points; the current fold and every
> effort window at/after the baseline are unaffected. Read paths are also
> bounded SQL-side now (`facts_for_measure_in_stream`, `pathless_scalar_facts`,
> `representative_facts_by_slice`, pinned findings via `facts_for_captures`) —
> never "load the whole measure history and filter in Rust" on a hot path — and
> the `EffortMetricsBlock` refetch is debounced (closed-long-ago efforts stop
> listening entirely; the OTLP token tick fires `MetricSamplesChanged` every
> ~10s while an agent runs).
>
> **Every `metricSamplesChanged` listener needs that debounce — it bit twice
> (tsk91).** `RecordedMetricsPage` + `MetricsExplorerPage` reloaded un-debounced,
> so oxplow burned **~20 CPU-seconds per agent tool call** (bursting to ~500% /
> ~200 threads, profiled straight to `row_to_fact_row`): a reload is one
> `listMetricSamples` per catalogued metric, fired as ~40 concurrent blocking
> reads, and **each walks its measure's whole history** — `oxplow.test_case` is
> ~235k facts (+~5k per `test:collect`) and yields ~118 points, one per capture.
> Both pages now carry the same 2.5s trailing debounce.
>
> ⚠️ **The debounce is mitigation, not the fix.** Ticks are ~10s apart, so it
> only coalesces a turn's burst; one full reload per tick remains. The defect is
> the read: **a series wants the newest N captures' facts, not the measure's
> whole history** (`facts_for_captures` exists), and the range/branch filters are
> applied **client-side in JS** after fetching — the same "load everything, filter
> after" sin one layer out. `list_metric_samples` takes no range arg.
>
> **Gauges must be able to FINISH a whole-tree scan, and a failure must be seen.**
> The `SandboxBudget` default (5s) is sized for a report parser over one file. A tree
> gauge tree-sitter-parses the *whole tree* per run, so gauge runs get their own
> ceiling (`GAUGE_TIMEOUT`, 120s). Under the old 5s budget the broad-query gauges
> timed out on every full-tree run and wrote **nothing** — `oxplow.ts.console_calls`
> and `oxplow.ts.ts_ignore` had produced **zero facts since the project was indexed**,
> against a repo with 137 console calls, and the only trace was a `tracing::warn`
> (tsk47). A failing gauge now records a **`status='failed'` capture** (with the
> error and the fingerprint), and a whole-tree sweep is a tracked
> `BackgroundTaskKind::Metrics` task with per-gauge progress that **fails** if any
> gauge failed (tsk48) — so "why is oxplow pegging a core" and "is this metric
> trustworthy" both have answers.
>
> ⚠️ **Non-`done` captures are invisible to every fold** (`c.status = 'done'` in
> `latest_tree_facts` / `latest_subject_facts` / `scanned_paths_for_captures`). This is
> load-bearing: a failure capture carries **no facts**, and on a full-tree snapshot it
> restates *every path* — so if the fold counted it, one timeout would supersede
> everything and silently zero the metric. Worse than the bug it reports.
>
> `needs_tree_baseline` asks ONE question **per gauge** (`gauges_needing_baseline`):
> *does this gauge have a completed `scan_kind='full'` capture at its current logic
> fingerprint?* (`SqliteFactStore::has_full_capture`; fingerprint =
> `gauge_fingerprint` — xxh3 of script text + runtime/input/args + `emits`, stamped
> on every capture as `metric_capture.producer_version`, V56). That single check
> covers BOTH a fresh/never-baselined gauge (incl. one stuck on deltas because its
> full scan used to time out, tsk47/tsk49) AND a script change since the last
> baseline (a full capture at stale logic carries the old fingerprint, tsk45). An
> unfingerprintable script matches any-version — one full capture ever.
>
> **Per-GAUGE, not per-measure, is load-bearing (tsk49).** `oxplow.ast_hit` is one
> measure shared by 10 idiom gauges (sliced by `rule`), so "does the measure have
> facts" says nothing about one gauge — a delta-only gauge looks done because a
> *sibling* filled the measure. That is exactly how `oxplow.ts.console_calls` read
> empty for weeks (137 real calls): `unsafe_blocks` completed its full-tree scan, so
> `ast_hit` wasn't empty, so the old measure-level check never re-baselined the heavier
> TS gauges that had only ever run on deltas.
>
> **(2) is not optional either.** A gauge's facts are only as good as the code that
> computed them, so a script change makes them stale but *not* empty. Without the
> fingerprint a metric fix **silently no-ops** — you correct the query, the number
> doesn't move, nothing says why (tsk44→tsk45: teaching `repo_allow.star` to match
> inner `#![allow(...)]` changed nothing until the captures were hand-deleted).
> Re-baselining restates every path, so the fold supersedes the stale facts — no
> deletes, history preserved.
>
> `per-path` today: `oxplow.ast_hit`, `oxplow.complexity`, `oxplow.fn_length`,
> `oxplow.parameter_count`, `oxplow.todo` (+ any project measure a snapshot gauge
> emits per-file facts on — `scaffold_metric` sets it automatically). Validated in
> config + `CaptureScope::parse`, deliberately **NOT** a DB CHECK: the
> `temporal_semantics` CHECK is exactly why adding a value *there* would need a
> `measure` table rebuild, which fires `fact.measure_id ON DELETE CASCADE` and wipes
> every fact (see V52).

- **`measure`** — the namespaced catalog of *fact types*: `key` (`oxplow.*`
  reserved), `title`, `unit`, `subject_kind` (the grain), `capture_scope`
  (`complete` | `per-path`, V54 — see the box above), `temporal_semantics`
  (`additive` | `semi-additive` | `non-additive` — additivity **over time**:
  tokens additive; complexity + test/lint SNAPSHOTS semi-additive (a run
  replaces the last — V47/tsk42 fixed test_case/lint_hit from V43's wrong
  `additive`); a **level ratio** whose every capture restates the value
  (coverage) is *also* semi-additive — the headline is the latest capture's
  Σn/Σd, not a history blend (V50/tsk13 fixed coverage from V43's wrong
  `non-additive`); only the **accumulating** mean-across-closes ratios
  (cycle_time, task_effort — one observation per close, Σ over all captures =
  the mean) are `non-additive`),
  `scope`, `description`. (`component_role` is a **dead** V43 column, tsk15 —
  never read; ratio components ride per-fact num/den. Its Rust plumbing + config
  wiring are removed; the column itself stays inert (`DEFAULT 'none'`) because a
  `DROP COLUMN` isn't safe — a CHECK constraint plus the `fact→measure` CASCADE
  under `foreign_keys = ON` would wipe the facts on a table rebuild.)
  Seeded built-ins: `oxplow.complexity`, `oxplow.fn_length`,
  `oxplow.parameter_count`, `oxplow.todo`, `oxplow.coverage`, `oxplow.test_case`,
  `oxplow.lint_hit`, `oxplow.duplicate_lines`, `oxplow.tokens`,
  `oxplow.cycle_time` (V43), plus `oxplow.ast_hit` (V45 — a per-file AST idiom
  occurrence; the per-language gauges emit facts on it, distinguished by the
  `oxplow.rule` dim; see the code-gauge section, tsk30), plus
  `oxplow.effort_test_outcome` (V53, tsk38 — a per-effort-close scalar the
  lifecycle producer materializes; the four `oxplow.tests.{failed_at_close,
  peak_failed,distinct_failed,red_runs}` specs slice it by `oxplow.tests_stat`.
  Non-additive like `cycle_time` — Σn/Σd = mean per effort — because the
  "within-effort" views (max/distinct/red-run count) can't be a plain spec over
  the raw per-case facts; see the producer table row).
- **`dimension`** — the namespaced slice-axis catalog: `key`, `label`,
  `value_type`, `subject_kind`, `vocabulary_json`, `scope`, `promoted` (whether a
  generated column + expression index exists). Seeded: `oxplow.language`,
  `oxplow.severity`, `oxplow.status`, `oxplow.branch`, `oxplow.model`,
  `oxplow.agent`, `oxplow.package`, `oxplow.test_suite` (V43), `oxplow.rule`
  (V45 — the lint/idiom name; the engine reads it off the fact's `rule` column),
  `oxplow.tests_stat` (V53 — which per-effort test-outcome scalar a
  `oxplow.effort_test_outcome` fact is: `at_close`/`peak`/`distinct_failed`/
  `red_runs`, tsk38).
  **Declare-to-collect**
  (planned, tsk17): a fact may only be emitted on defined measures/dimensions;
  historical facts carrying a now-undefined dim are kept but hidden as a slice
  axis (the axis list is catalog-driven).
- ~~**`subject`** — the subject hierarchy (file→package→repo) for roll-ups.~~
  **Dropped in V52 (tsk15)** — never got an INSERT/SELECT; the rollup reads
  package-from-path off the fact directly.
- **`metric_capture`** (the renamed/generalized `metric_run`) — the **one context
  row**: it holds ALL the "when/where/who/trust" metadata so it isn't duplicated
  on every fact. `producer`, `trigger`, `status`/`error`, `scope`; when
  `captured_at`/`ended_at`; where `snapshot_id`/`closest_git_version`/
  `git_version_exact`/`branch`/`basis_ref`; who `stream_id` (NOT NULL, the CASCADE
  scope) / `thread_id` / **`effort_id`** (nullable, `ON DELETE SET NULL` — the
  *producing* effort, stamped only when unambiguous; ledger-backfilled otherwise);
  trust `provenance`/`source`. **Captures are durable** (they carry the facts'
  context — no independent sweep).
  > **Stamp `closest_git_version` on every capture you add (tsk95).** A result is
  > about a **code state, not a branch name**, and the version is the *only*
  > ancestry material the fold can use (tsk97). It is **NOT backfillable** —
  > which commit a past run tested is unrecoverable, so an unstamped capture is
  > permanently ancestry-blind. This was missed for 125 `test_case` captures
  > because version resolution rode on `GaugeRunContext`'s `snapshot_id` and test
  > runs carry no snapshot. Use `file_ref_version::resolve(store, dir, snap)`: a
  > snapshot with its own commit reads `git_version_exact = true`, otherwise it
  > falls back to HEAD with `exact = false`. **Dirty is the normal case** (the
  > agent edits, then runs tests), which is exactly what the
  > `closest_git_version` + `git_version_exact` pair is for — don't add a third
  > field.
- **`fact`** — the durable atomic measurement (folds `metric_sample` +
  `metric_finding`): `capture_id` **NOT NULL** (→ all context via the capture),
  `measure_id`, `value`, `numerator`/`denominator`; subject `subject_kind`/
  `subject_ref`/`path`/`line` (location-at-capture); reported finding metadata
  `severity`/`rule`/`detail` (null for pure measurements); `dims_json` (long-tail
  dims). **No when/where/who columns** — those are the capture's.
- **`metric_cube`** + **`metric_live_fact`** + **`metric_cube_state`**
  (`V62__metric_cube.sql`, tsk96) — the **aggregate cube**: the materialized fold.
  See the box below.

> ### ⚠️ The cube is an accelerator, NEVER a replacement for the facts (V62, tsk96)
>
> **Why it exists.** For a partial-scope measure the read is a stateful replay, so
> one sparkline over `oxplow.test_case` decoded 240k facts to emit 125 points —
> ~1M decodes per refresh across the 5 test specs, every ~10s. `metric_cube`
> stores the fold's *output*: one row per `(measure, capture, promoted dims)`
> holding the **decomposable** components `count/sum/min/max/numerator/
> denominator`. A read becomes a GROUP BY over ~152 rows. *You cannot GROUP BY a
> fold; you can GROUP BY a pre-folded cube.*
>
> **The decomposability contract.** `metric_engine::Cell::project` and
> `aggregate_facts` are two sides of one identity — bucket, aggregate, merge must
> equal aggregate-all — pinned by
> `cube_cells_reaggregate_to_the_same_value_as_the_raw_facts`. **Edit them
> together.** Every aggregation in the catalog is decomposable (sum/avg/count/max/
> ratio); **`last` is not** (merging destroys the ordering it means) and
> `project` returns `None` so the read falls back rather than guesses. Ratio
> components accumulate **only from facts carrying BOTH** — Σn/Σd, never a mean of
> percentages, and never a naive `SUM(numerator)`.
>
> **The cube is a LOSSY projection, and that lossiness IS the speedup.** It drops
> the **subject axis** and nothing else. So these reads stay on the raw facts
> *permanently and by design* — this is not a temporary fallback to be removed:
> - **value-threshold specs** — `oxplow.high_complexity_fns` (`min_value: 11`),
>   `oxplow.long_functions` (`min_value: 61`). The cube summed those values away;
>   answering them would need a bucket per distinct value — the fact table again.
> - **findings / drill-in** — "which test, which file, which line" *is* the
>   subject axis.
> - **`group_by` on an unpromoted dim** — caller-supplied at runtime;
>   `group_by = subject` has zero reduction.
>
> This is ordinary **aggregate navigation** (Kimball): an aggregate fact table
> never replaces the base fact table; the query layer picks the smallest table
> that can answer. The fact path cannot rot from disuse — it serves everything
> except the handful of cube-eligible sparklines, and it is the **oracle** the
> equivalence test checks the cube against.
>
> **⇒ The cube is DISPOSABLE.** It is 100% derivable from facts; delete every row
> and you lose only speed. **Never let a read depend on it for data**, and never
> "fix" a wrong cube number by writing data the facts don't have.
>
> **Why a durable live-state table** (`metric_live_fact`) rather than delta
> arithmetic on the previous row: `state[N] = state[N-1] − restated + facts`
> decrements fine for count/sum/num/den, but **min/max are not decrementable** —
> evict the subject holding the max and it is unrecoverable from the aggregate.
> `oxplow.tests.slowest_ms` is a `max` over a per-subject measure, so a
> delta-maintained cube would have been **silently wrong** for it. Re-aggregating
> live state is correct for every aggregation by construction, and it is what
> turns a replay into an increment.
>
> **The watermark** (`metric_cube_state`) exists because "no cube rows for capture
> N" is otherwise ambiguous: state legitimately empty at N (a real value-0 point)
> vs N not cubed yet (fall back). Conflating those is how a materialized read
> reports 0 instead of admitting it doesn't know. It also makes the build
> **crash-safe**: the cube is written outside the fact-insert transaction, so a
> torn write just leaves the watermark un-advanced — reads fall back and the next
> build re-runs whole captures, which is idempotent (evict+insert *replaces* a
> subject's facts).
>
> **Deleting captures invalidates the cube** (tsk100). `prune_dominated_tree_captures`
> drops per-path captures and their facts cascade; `metric_live_fact` cascades with
> them (FK on `fact_id`) so live state self-heals, but **`metric_cube` rows are
> frozen at build time and don't**. Usually they'd still agree — the baseline
> restated those paths already — but not for a path the sweep never restated (a
> changed gauge glob: neither scanned nor tombstoned), which stays live until the
> prune deletes it. So the prune **invalidates that stream's cube in the same
> transaction**, rather than reasoning about which prunes are safe; the next build
> re-folds. **Only when it actually dropped something** — `rebuild_metric_baseline`
> prunes on every boot, so unconditional invalidation would wipe a healthy cube each
> start and turn the fix off for nothing. Any future code that deletes captures or
> facts owes the cube the same treatment.
>
> **The grain's floor is the CAPTURE.** Never aggregate coarser (per-day,
> per-commit): a capture *is* one scan/run, so `snapshot_id`/`effort_id`/
> `thread_id`/`branch`/`closest_git_version`/`stream_id` stay reachable through
> the JOIN and within-effort deltas keep working. Branch/thread/stream remain
> **dimensions you can group by, never partitions that hide rows**.
>
> **`dimension.promoted` = the cube's grain** (tsk28's flag, inert until V62).
> `oxplow.status` is promoted (cardinality 2; 125 → 152 rows, and it is what
> `tests.passed`/`tests.failed` filter on). `oxplow.test_suite` is not
> (cardinality 234 ⇒ 18,918 rows, for slicing no spec asks for). **Promoting a dim
> later is a cube rebuild, not a schema change** — the raw facts always keep every
> dim, so nothing is foreclosed.
>
> ### Where the code lives (`metric_cube.rs`)
>
> Both sides live in **oxplow-app**, not oxplow-db: bucketing needs `dim_value` +
> `Cell`, and oxplow-db can't depend on oxplow-app. Doing it in SQL would mean a
> **second dim-extraction implementation free to drift** from the read's. One
> implementation, called from both sides, is the point — and it's why the build
> runs outside `record_facts`' transaction (safe: see the watermark, above).
>
> - **`MetricCubeBuilder::build_measure`** — dispatches on scope to **two build
>   rules, deliberately not merged** (tsk99):
>   - **partial** (`build_stream`) — folds each capture after the watermark: evict
>     what it restates → insert its facts → re-aggregate the **whole live state**.
>   - **complete** (`build_stream_complete`) — a GROUP BY over the capture's **own**
>     facts. No `metric_live_fact`, no eviction, no reach-back: every capture
>     restates the whole population, so `state[N] = facts(N)`.
>
>   They look mergeable and are not. A state fold evicts **per producer**, which
>   would leave another producer's earlier facts standing and make `agg(state) !=
>   agg(the capture's own facts)` — merging them silently changes every
>   complete-scope number. Both advance the watermark the same way, and **backfill
>   is the same loop from an empty watermark** — never a second SQL fold.
> - **`cube_series`** — the read, for **both** scopes. Returns **`None` for anything
>   it can't answer exactly**, and the caller falls through to the facts.
>   Eligibility: decomposable agg, no `min_value`, filter/`group_by` dims all
>   promoted, and **every capture ≤ the watermark**. The capture list and the
>   filter-narrowed producer set come off **captures and the cube, never facts** —
>   deriving them by scanning facts is the decode being removed, so doing it there
>   fixes nothing.
>
>   The scopes differ in exactly two places, both in the ungrouped branch: an
>   **empty partial** capture emits an explicit **0** (empty live state is a real
>   zero); an **empty complete** capture emits **nothing** and is left to
>   `splice_zero_points`, because `aggregate_series` only ever emitted points for
>   captures that had matching facts. Complete scope then applies
>   `splice_zero_points` — the **same function** the fact path calls (tsk44), not a
>   copy. Partial deliberately skips it: an empty partial capture restated nothing,
>   so it means "nothing changed", not "the repo is zero".
> - **`run`** (spawned in `boot.rs`) — backfills, then keeps up off
>   **`MetricSamplesChanged`**, the one signal every recording site emits
>   (`collection`, `metrics_service`, `task_service`, `token_usage`, MCP). Hooking
>   individual `record_facts` calls would mean five crates to keep in step. Bursts
>   coalesce; failures are logged, never propagated.
>
> **Measured on the real DB (512k facts).** The 5 test specs: **9.26s → 70ms
> (~131×)**. All **68** specs (both scopes, after tsk99): **11.53s → 1.03s**, with
> **zero divergence** from the fact path on any of them. Backfill ~25s once, in the
> background. *That 9.26s every ~10s was the CPU burn.*
>
> **42 of 68 specs are cube-served**; the other 26 decline, and every one is an
> expected class — 18 filter `dim_eq` on an **unpromoted** dim (`oxplow.rule` ×10,
> `oxplow.token_kind` ×4, `oxplow.tests_stat` ×4), 2 filter on `severity`
> (unpromoted), 2 are `min_value` thresholds (permanent, by design), and 4 measures
> have no facts at all. Those first 20 are a *grain* choice, not a limit — all four
> dims are low-cardinality, so promoting them would cube those specs too (tsk101).
> Verify a decline is one of these classes before assuming the cube is working.
>
> **The equivalence gate.** Tests take the fact-served oracle **before** the build
> — after one, `series_for_spec` reads the cube, so a later oracle is just the cube
> confirming itself. `assert_cube_answers` **expects `Some`**: without that, a
> regression silently disabling the cube would leave every equality passing
> vacuously. Both properties were verified by mutation (bucket the capture's own
> facts instead of live state → reads 10 where the fold reads 11; force `None` →
> three tests fail rather than pass green).
- **`metric_spec`** (`V44__metric_spec.sql`, tsk29) — the **metric-as-a-spec**
  catalog (the third catalog beside measure + dimension). A metric is NOT a stored
  sample stream (V38's `metric_definition` *owned* `metric_sample` rows); it is a
  **spec computed over facts at read time**: `key` (`oxplow.*` reserved), `title`,
  `unit`, `source_measure` (the measure whose facts it aggregates; NULL for a
  formula metric), `aggregation` (`count`|`count_distinct`|`sum`|`avg`|`min`|`max`|
  `last`|`p95`|`ratio` — how source facts combine *within* a capture; cross-time
  collapse is the source measure's `temporal_semantics`, not stored here),
  `filter_json` (the conjunctive predicate that turns a raw measure into a
  count-over-threshold), `formula` (derived-metric spec referencing other metric
  keys; NULL for a base), `sliceable_dims_json`, presentation
  - A project `key:`-defined metric may set `source_measure` to a **built-in**
    measure to add a new aggregation over facts a bundled gauge already emits —
    no new gauge, no collection. E.g. `repo.complexity_max` = `max` over
    `oxplow.complexity`, `repo.fn_length_max` = `max` over `oxplow.fn_length`
    (the project key just can't reuse the reserved `oxplow.` namespace).
  `direction`/`target`/`warn_at`/`fail_at`/`display_kind` (`gauge`|`findings`|
  `test`|`coverage`|`event` — read-time only; severity/threshold-state are DERIVED
  from `value` × these, never stored on a fact), `scope`/`category`/`language`.
  **Additive** beside the old `metric_definition` (still FK-referenced by the V38
  `metric_sample`/`metric_finding`); the retire migration (tsk20) drops the V38
  cluster once reads flip (tsk26). The migration seeds no rows; the **built-in
  specs** are seeded from Rust in `seed_catalog`: the code-metric specs
  (`oxplow.high_complexity_fns` / `long_functions` / `fn_count` / `todos` — a
  `count` over its measure, thresholds via `min_value`; `builtin_metric_specs`,
  tsk23) and the per-language idiom specs (`oxplow.rust.unsafe_blocks` etc. — a
  `Sum(oxplow.ast_hit)` filtered by `dim_eq(oxplow.rule, …)`; `builtin_ast_specs`,
  tsk30). Config/global spec seeding lands with the read-flip (tsk26).

`SqliteFactStore` API: `upsert_measure`/`get_measure`/`list_measures`,
`upsert_dimension`/`list_dimensions`, `upsert_spec`/`get_spec`/`list_specs`,
`record_capture`, `record_facts(capture, facts)` (atomic — inserts the capture,
backfills `capture_id` into every fact, commits together; **idempotent** when
the capture carries an `idempotency_key` — a second write with the same key is a
no-op that returns the existing id, so a replayed report never double-counts,
tsk14/V51. `metric_capture.idempotency_key` is nullable with a partial unique
index; the report ingests set it via `CollectionService::ingest_idempotency_key`
= hash(producer + git version + snapshot + verbatim payload). Keyless captures —
gauges, tokens, lifecycle — always insert fresh), `get_capture`,
`facts_for_measure` (joined to the capture for the time/version/effort spine),
`facts_for_captures` (the attribution-by-claim read), `aggregate_ratio`.

### The aggregation engine — `crates/oxplow-app/src/metric_engine.rs`

`MetricEngine { facts: SqliteFactStore }` turns facts into metrics at read time:
- `Aggregation` (`Count|Sum|Avg|Min|Max|Last|Ratio`, `parse`),
  `Temporal` (`Additive|SemiAdditive|NonAdditive`), `FactFilter`
  (`min_value` / `severity` / `dim_eq` — the count-over-threshold + slice filters).
- Pure cores: `aggregate_series(facts, agg, filter, group_by)` → one `SeriesPoint`
  per capture (preserving time order), optionally one series per group-by
  dimension value; `range_value(series, temporal)` collapses a series to one
  number the additivity-correct way. An `avg` point carries `(Σvalues, count)`
  as its ratio components so the non-additive collapse (Σn/Σd) yields the mean
  across ALL facts — the V47 mean-across-closes measures (cycle_time,
  task_effort) would otherwise collapse to a den=0 → 0.0 headline; `compute_rollup(facts, dimension, temporal,
  current_caps)` → `RollupRow`s, additivity-aware like `range_value` (tsk41) and
  scoped to the CURRENT captures (tsk44): semi-additive → only facts in the
  latest capture per (stream, producer) (`current_capture_ids` — else a deleted
  file's stale last fact haunts the breakdown forever), latest-per-subject,
  summed per dim value — **unless** the facts carry ratio components (a level
  ratio like coverage, tsk13), in which case the per-group value is Σn/Σd, never
  a sum of per-file percentages; additive → EVERY fact counts (tokens by model
  is a running total, not the last turn); non-additive → current captures,
  latest per subject, per-group Σnumerator/Σdenominator, never a naive
  sum/average of percentages. The rule is uniform: **any** group whose facts
  have Σden≠0 collapses to Σn/Σd regardless of temporal class. `dim_value` reads the `severity`/`rule` columns and
  `package`-from-path directly, else `dims_json[key]`; `oxplow.language` /
  bare `language` alias each other (the gauge scripts emit the conformed
  namespaced key; pre-rename facts and the Explorer's declared sliceable_dims
  use the bare form). `FactRow` carries the
  capture's `producer` for exactly this scan-currency logic.
- Async wrappers `MetricEngine::series(measure_key, agg, filter, group_by)` and
  `rollup(measure_key, dimension)` fetch a measure's facts and aggregate
  (`rollup` parses the measure's `temporal_semantics`, erroring on a malformed
  value rather than guessing). **Zero-fill (tsk44):** a scan that found nothing
  writes an EMPTY capture (see the producer section), and `series` splices a
  value-0 point for every such capture of the metric's producers (count/sum
  aggregations, ungrouped) — so a count metric drops back to zero after the last
  offender is fixed instead of showing the previous scan forever. Producers are
  derived from the facts that ever matched the spec's filter
  (`captures_for_producers` on the fact store fetches their captures).
- **Spec-driven reads** (tsk29 — a metric *key* → its computed result): given a
  `MetricSpec`, `series_for_spec(spec, group_by)` / `rollup_for_spec(spec, dim)` /
  `headline_for_spec(spec)` resolve the spec's `source_measure` + `aggregation`
  (`FactFilter::from_json` parses `filter_json`) and run the pure cores;
  `headline_for_spec` collapses across time per the *source measure's*
  `temporal_semantics`. Each has an `_in_stream` variant (`series_in_stream` /
  `series_for_spec_in_stream` / `headline_for_spec_in_stream` — the series
  sibling of the tsk46 rollup scoping): unscoped, per-worktree scans interleave
  into one timeline and a semi-additive headline flips to whichever worktree
  scanned last; the zero-fill only splices the scoped stream's empty captures.
  `headline_from_series` collapses an already-computed series so a summary read
  pays the fact load once. **Percent presentation:** a `ratio` spec with unit
  `%` reads ×100 (`spec_value_scale`) — the facts carry raw components
  (covered/instrumented lines) and the engine derives 0..1, but the spec's
  unit/thresholds and the per-fact `value` column are 0..100; series/rollup/
  headline agree with them (the raw num/den stay on the point). Measure-level
  reads return the raw fraction. A formula metric (no `source_measure`) yields empty/None;
  an aggregation the engine can't yet compute (`count_distinct`/`p95`) or a
  malformed `filter_json` is a surfaced `DomainError::Invalid`, never a silent
  wrong number. This is the bridge the read flip (tsk26) and UI (tsk18) consume.
- **T-C1 plumbing (additive, tsk26 prep):** `SeriesPoint` carries `branch` +
  `provenance` (one capture → one of each, taken from the bucket's spine in
  `aggregate_series`); `findings_for_spec(spec, capture_id?)` projects a spec's
  filtered facts as `FactFinding`s (the offenders drill-in that replaces the baked
  `metric_finding` — severity is the fact's reported severity or, absent one,
  DERIVED via the shared `threshold_state(direction, value, warn_at, fail_at)`,
  lifted here from `collection.rs`); `dim_value` gains branch/subject/model
  **pseudo-dims** (off the capture/fact spine, not `dims_json`) so `group_by` is
  uniform server-side. `SqliteFactStore::captures_for_effort(effort_id)` returns
  an effort's captures (the attribution-by-claim spine for T-D). These stay
  non-`Type` (out of `bindings.ts`) until T-C3 wires the IPC.

### Producers — facts on the capture spine (the ONLY write since T-E2)

Each producer writes atomic facts through `record_facts` (a capture + the
facts). The legacy V38 sample/finding/run/definition writes were removed in
T-E2 (tsk49); producers emit `MetricSamplesChanged` after their capture write.

**Collection gate (tsk31).** Before writing, each base-data producer checks
`fact_store.measure_has_active_spec(<measure>)` and skips when no *enabled* metric
consumes that measure (the spec table = the enabled set after `seed_catalog`'s
reconcile). So disabling every metric over a measure stops its collection:
`oxplow.tokens` (all `agent.tokens.*` off), `oxplow.test_case` (all
`oxplow.tests.*` off), `oxplow.lint_hit` (both `oxplow.analysis.*` off),
`oxplow.coverage`, `oxplow.turn`, `oxplow.nudge`, `oxplow.cycle_time` /
`oxplow.task_effort`. **Tests keep their run record** even when the metric is off
— a measured run whose `oxplow.tests.*` are all disabled records under the
record-only `test-run` producer (no metric facts) so effort-review still sees the
run. Code gauges need no gate — `resolved_gauges()` already elides a disabled
gauge (it never runs). Test fixtures that exercise a producer must seed the
producer specs (as boot does) or the gate stays closed.

Landed:

| producer | where | facts |
|---|---|---|
| tokens — OTEL (tsk22) | `token_usage.rs::ingest_otlp_tokens`, fed by the control-plane `POST /v1/metrics` OTLP receiver | PER-KIND facts on `oxplow.tokens` (one input + one output per model, sliced by the `oxplow.token_kind` dim), producer `otel-tokens`, one capture per OTLP export with an `idempotency_key` over the raw body (SDK-retry-safe). Attribution rides the `X-Oxplow-Thread`/`X-Oxplow-Stream` OTLP headers the spawn path injects; effort via `find_single_open_for_thread`. `agent.tokens.total` sums both kinds; input/output specs filter by `token_kind`. **Source of the token facts** — see [OTEL token tracking](#otel-token-tracking-tsk22) |
| prompt-cache tokens (tsk73) | same ingest, same capture | Cache kinds (Claude `cacheRead`/`cacheCreation`, Codex `cached_input` → cache_read) land on **`oxplow.cache_tokens`** — a SEPARATE measure, because `agent.tokens.total` is an unfiltered sum over `oxplow.tokens` and cache facts there would silently change its meaning. Plus one per-model **`oxplow.cache_usage`** ratio fact per export: `num = cache_read`, `den = input + cache_read + cache_creation` (prompt-side; output can't be cached) — non-additive, so the cross-time collapse is the cumulative Σn/Σd hit ratio (`agent.tokens.cache_hit_pct`). An export with NO cache telemetry emits no ratio fact (an agent that doesn't report cache reads as "no data", not 0%). **Token-denominated only — never dollars**: the API returns token counts; a locally maintained price table is invalid by construction (Claude Code's OTEL `cost.usage` *estimate* would be the only defensible future dollar source, not ingested today) |
| effort token spend (tsk73) | `task_service.rs::project_effort_lifecycle_metrics` (the close-time sub-producer beside effort_test_outcome) | one **`oxplow.effort_tokens`** fact per closed effort: Σ of ALL token kinds from its effort-stamped otel captures (num=value/den=1, non-additive → `task.tokens` reads the MEAN tokens per close — the cost of a unit of work, in tokens). No fact when the effort has no token captures (unmetered ≠ zero) |
| wasted tokens (tsk77) | close-time producer + `collection.rs::record_token_waste_for_reverts` (fires on any landed commit incl. `git revert`, via `detect_git_revert` — revert never says "commit") | **`oxplow.token_waste`** is an append-only ratio measure with two writers: a metered CLOSE emits (num 0, den = the effort's spend, value 0) — rides inside the effort_tokens gate since the denominator IS that spend — and a detected revert emits (num = spend, den 0, value = spend) for the ONE closed effort whose window contains the reverted commit (`This reverts commit <sha>` trailers in HEAD; 0/ambiguous candidates → no attribution; idempotency key `token-waste:<effort>` → one waste fact per effort ever; commit times are seconds-granular so containment spans the whole second). `task.tokens.wasted` = SUM over values (closes are 0); `task.tokens.wasted_pct` = ratio Σn/Σd = wasted ÷ all metered spend. V1 is coarse: one reverted commit flags the effort's FULL spend. Pre-V61 closes never entered the denominator |
| effort steering (tsk76) | same close-time producer | one **`oxplow.effort_steering`** fact per closed effort (num=value/den=1, non-additive → `task.steering` reads the MEAN per close — the autonomy number, lower = more autonomous): user prompt submissions (`agent_turn` rows opened in the effort window, newest-1000 scan) + Stop-hook nudges (Σ of the effort's `oxplow.nudge` facts) + non-`agent`-authored comment threads opened in the effort's thread during the window. **Zero IS emitted** — a fully autonomous effort is real data. Interrupts are NOT counted (nothing records them yet). Needs `with_steering_sources` (agent-turn + comment stores) wired, as boot does |
| effort time-to-green (tsk76) | same close-time producer (shares the `oxplow.test_case` read with effort_test_outcome — one fetch when either gate is open) | one **`oxplow.effort_time_to_green`** fact per closed effort: wall-clock ms from the FIRST red run to the first green after it (pure `test_outcome::time_to_green_ms` over per-capture red flags + `captured_at`). Only emitted when that red→green transition exists — always-green or never-recovered is "no data", not a zero. `effort.time_to_green_ms` reads the mean |
| turns — transcript (tsk22) | `token_usage.rs::record_token_metrics`, from `on_stop` | a `oxplow.turn` fact per model per Stop (turn COUNT = genuine user prompts). The transcript path **no longer projects `oxplow.tokens`** (OTEL owns those); it still records the per-turn `agent_token_usage` rows (with prompt text OTEL lacks). The `parse_claude_turns`/`parse_claude_usage` dedupe-by-`message.id` fix (tsk22) removed the ~2–3× overcount from Claude repeating a message's `usage` on every content-block line |
| effort lifecycle (T-B) | `task_service.rs::project_effort_lifecycle_metrics` | one `oxplow.cycle_time` fact per close (subject=effort) + one `oxplow.task_effort` fact (subject=task, the efforts-so-far redo signal); both carry `numerator=value, denominator=1` (the measures are non-additive per V47, so Σn/Σd across time = the MEAN across closes, tsk42); capture **stamps `effort_id`** (unambiguous — this producer knows the exact effort). **Also (tsk38)** emits four `oxplow.effort_test_outcome` facts per close, sliced by `oxplow.tests_stat` — `at_close` (failed count of the last run = quality gate), `peak` (max failed in any run), `distinct_failed` (distinct cases red in ≥1 run), `red_runs` (# runs with ≥1 failure). Computed by the pure `test_outcome::{runs_from_case_facts, compute_effort_test_outcome}` from the effort's `oxplow.test_case` facts (grouped per capture): these "within-effort" aggregates are **not expressible** as a spec (the engine's temporal collapse is only sum/last/Σn÷Σd), so they're materialized here. Gated by `measure_has_active_spec("oxplow.effort_test_outcome")` |
| nudges (T-B) | `collection.rs::project_nudge_metric` | one `oxplow.nudge` event fact per fired nudge (value 1, subject=the nudge kind) — the `agent.nudges.fired` spec is `Sum(oxplow.nudge)` |
| lint hits | `collection.rs::mirror_analysis_metrics` | one `oxplow.lint_hit` fact per finding (severity/rule/detail columns + file location) |
| coverage | `collection.rs::observe_coverage` | one `oxplow.coverage` fact per file (value=line-%, num/den=covered/instrumented → engine re-derives Σcov/Σinstr) |
| test cases | `collection.rs::record_test_run` | one `oxplow.test_case` fact per case, status as the `oxplow.status` dim (+ `oxplow.test_suite`). MCP-asserted counts (no report) synthesize status-sliced facts (no case identity). A report-less, count-less run records its capture under the **`test-run`** producer — a run RECORD, not a measurement: an empty `tests` capture would read as "found 0 tests" to the zero-fill/currency logic and collapse the semi-additive `oxplow.tests.*` timeline |
| duplication | `oxplow-rpc/…/code_quality.rs::run_duplication_scan_at` | one `oxplow.duplicate_lines` fact per duplicate block (value=line count, subject=`path:start-end`, peer side in `detail`); capture stamped with the **primary stream** (a scan has no natural stream) + tree `basis_ref`. A zero-hit scan still writes its EMPTY capture (tsk44 currency) — else the last non-empty scan's blocks stay "current" forever |
| code gauges | `metrics_service.rs::run_one_gauge` → `record_gauge_facts` (tsk23) | the bundled code gauges emit a `facts` channel: one fact **per function** on `oxplow.complexity` (high_complexity_fns) / `oxplow.fn_length` (long_functions) / `oxplow.parameter_count` (fn_count), and one per marker on `oxplow.todo` (todos) — the raw grain, for **every** item, not just the offenders the baked count reports |
| per-language idiom gauges | same path (tsk30) | the ~10 idiom gauges (`oxplow.rust.unsafe_blocks`, `oxplow.ts.any_usage`, `oxplow.csharp.empty_catch`, …) emit one **per-file** `oxplow.ast_hit` fact (value=the file's count, `rule`=the idiom slug, dims carrying the conformed `oxplow.language`); the metric is a `Sum(oxplow.ast_hit)` spec filtered by `dim_eq(oxplow.rule, <slug>)` (`builtin_ast_specs`) |

#### OTEL token tracking (tsk22)

Token facts come from **OpenTelemetry**, not transcript parsing. The old
Stop-hook transcript parse overcounted ~2–3× (Claude writes one JSONL line per
content block and repeats the message's cumulative `usage` on each; the parser
summed every assistant line) and was Claude-only + format-fragile.

- **Receiver:** the control plane hosts `POST /v1/metrics`
  (`oxplow-control-plane/src/lib.rs::handle_otlp_metrics`), behind the same
  bearer auth as `/hook`. It reads the `X-Oxplow-Thread`/`X-Oxplow-Stream`
  headers (attribution spine — one agent process per thread, so the headers are
  constant) and hands the raw protobuf body to `ingest_otlp_tokens`. Always
  answers a 200 OTLP ack (best-effort side-band; a non-2xx would make the
  exporter retry-storm).
- **Decode + map:** `oxplow-app/src/otlp_tokens.rs` decodes the OTLP protobuf
  (`opentelemetry-proto` crate) and `otlp_metrics_to_token_facts` projects both
  agents' token metrics into `TokenFact`s (pure + unit-tested):
  - **Claude** — `claude_code.token.usage` **counter** (delta temporality → each
    export is the increment), `type ∈ {input,output}` (cacheRead/cacheCreation
    dropped);
  - **Codex** — its `codex.sse_event` **log event** with
    `event.kind=response.completed` (tsk27, confirmed against Codex 0.142.0 via
    the tsk25 diagnostic — Codex points its single OTLP endpoint at us and sends
    token counts as **logs**, not a metric). Counts are per-request, so new
    input = `input_token_count − cached_token_count`, output =
    `output_token_count + reasoning_token_count`; `model` reads the record then
    the resource. `ingest_otlp_tokens` tries metrics-decode then logs-decode, so
    one endpoint accepts both agents. (A speculative `codex.turn.token_usage`
    *metric* mapper also exists, unemitted by 0.142.0 — kept as a defensive
    path.)
- **Launch wiring:** per-agent, injected at spawn (`terminal.rs`):
  - **Claude** (`claude_otel_env`, env — Claude has OTEL env support):
    `CLAUDE_CODE_ENABLE_TELEMETRY=1`, `OTEL_METRICS_EXPORTER=otlp`,
    `http/protobuf`, `OTEL_EXPORTER_OTLP_ENDPOINT` = the control-plane
    `otlp_base_url` (base; SDK appends `/v1/metrics`) threaded via
    `PluginRuntime`, + bearer + `X-Oxplow-*` headers.
  - **Codex** (`codex_otel_overrides`, `--config otel.*` — Codex has NO OTEL env
    vars): `otel.exporter.otlp-http.endpoint` = the **full** `<base>/v1/metrics`
    URL, `protocol="binary"` (protobuf), same bearer + `X-Oxplow-*` in the
    exporter's `headers` map.
  - **opencode** is not auto-instrumented (a user's own OTEL plugin pointed at
    the receiver still works).
  > **Codex confirmed live (tsk27):** a real Codex 0.142.0 run (via the tsk25
  > diagnostic) showed the `--config otel.exporter.otlp-http.*` injection works
  > (its exports reach us with the `X-Oxplow-*` headers), and that Codex's token
  > counts arrive as the `response.completed` **log event** — not the metric we
  > first guessed. `input_token_count` is the full request context (mostly
  > cache-read on later turns), hence the `input − cached` mapping. Claude's
  > metric path was confirmed live in the same run.
  > **Diagnostic (tsk25/26):** set `OXPLOW_OTLP_DEBUG=<file>` before launching
  > oxplow — the receiver appends a decoded dump of every OTLP export (metrics
  > AND logs: names/event-kinds + attributes) to that file
  > (`otlp_tokens::summarize_metrics_request`). Off by default.
  See `.context/agent-model.md`.

**Fact-attribution spine — `metric_capture.effort_id` (T-D prep, tsk37).** The
read-side effort attribution (T-D) resolves an effort's facts from *its captures*
(`captures_for_effort`). So the effort-scoped producers stamp `capture.effort_id`
at write time using the **same** resolution the run-ledger auto-claim uses —
`CollectionService::resolve_owning_effort(thread, task)`: a named task is
exact-or-nothing (`find_open_for_task`); an unnamed one claims only the single
open effort (`find_single_open_for_thread`), else stays null (deferred to
reconcile). Stamped by: **tokens/turns** (`token_usage.rs`, the effort resolved
once in `on_stop` and threaded to the capture), **tests / lint-hits / coverage /
nudges** (`collection.rs`), and **effort-lifecycle** (`task_service.rs`, which
knows its exact effort). The **snapshot code-gauge** captures are deliberately
NOT stamped — they're whole-tree scans whose baseline predates the effort, so
T-D's File family attributes them by *claimed files × time window*, not by
`effort_id`. `auto_attribute_run` now composes `resolve_owning_effort` +
`claim_run` (the `run:<id>` ledger write is unchanged) so the run claim and the
capture stamp always agree.

**Code-gauge unbake (tsk23) — the keystone, and the one non-mechanical producer.**
A gauge's `MetricReport` gained a third channel beside `samples` (baked headline)
and `findings` (offenders drill-in): `facts: [GaugeFact { measure, value, subject?,
path?, line?, dims? }]` — measure-bound atomics. `record_gauge_facts` resolves each
fact's `measure` against the catalog (**declare-to-collect**, decision #4: a fact on
an undefined measure is dropped with a `tracing::warn!`, never silently written) and
writes the resolvable facts under one capture — the **only** output now (facts-only,
T-C3b). A ZERO-fact run still writes its (empty) capture — "this scan ran and found
nothing" is the record the engine zero-fills a series from, so a count metric drops
back to zero after the last offender is fixed (tsk44; the analysis ingest records
its capture for a clean report the same way). The count-over-threshold headline is the **spec** (`builtin_metric_specs`),
and the equivalence test `code_gauge_facts_reaggregate_to_the_expected_headline`
pins `engine.headline_for_spec(spec) == the expected gauge total` for every bundled
code metric — the proof the inversion is faithful. (Strict `> N` in the old gauge
equals `min_value = N+1` on the integer complexity/length measures.) The 4 code
scripts are unbaked (no `tree:.`/`file:` samples); the baked write path is gone.

**Per-language idiom gauges (tsk30).** The same pattern extends to the ~10
per-language AST idiom gauges, but they don't have a natural per-subject measure —
so they share **one** generic measure `oxplow.ast_hit` (a per-file idiom
occurrence) and are told apart by the `oxplow.rule` dimension (the idiom slug,
carried on the fact's `rule` column via the new `GaugeFact.rule`). Each gauge emits
one per-file `ast_hit` fact (value=that file's count, `rule`=its slug) beside its
per-file sample; each metric is a `Sum(oxplow.ast_hit)` spec filtered by
`dim_eq(oxplow.rule, <slug>)` (`builtin_ast_specs`, seeded in `seed_catalog`). Idioms
sharing the measure never collide because `rollup_for_spec` applies the rule filter
**before** the per-subject rollup. `per_language_gauge_facts_reaggregate_to_the_baked_headline`
pins each spec's `Sum` to its baked headline. The `<slug>` in the script and the
spec MUST match (the equivalence test catches a drift → spec count 0 ≠ baked).

Wired into `Services` as `fact_store: Arc<SqliteFactStore>` +
`metric_engine: MetricEngine`; `TaskService`/`CollectionService`/
`TokenUsageService` carry the fact store; the duplication write lives in the rpc
layer (`svc.fact_store`). Still to come (see the epic's tasks): the **code-gauge
unbake** (per-function complexity/length/param + marker facts; count-over-threshold
becomes a metric spec — the design-heavy keystone).

**Producer specs (T-B).** Each always-on producer metric now has a `metric_spec`
(`producer_metrics.rs::builtin_producer_specs`, seeded in `seed_catalog` beside
the built-in gauge specs) — the aggregation it *is* over the measure its producer
emits facts on, with conformed dims (not extra measures) distinguishing variants:
token in/out slice `oxplow.tokens` by `oxplow.token_kind`; tests slice
`oxplow.test_case` by `oxplow.status`; analysis filters `oxplow.lint_hit` by
severity; coverage is a `ratio` over `oxplow.coverage`. `producer_spec_shape`
holds the `(source_measure, aggregation, filter)` per key. Equivalence tests pin
each spec's engine headline to the baked total (tokens, tests). New V46 measures:
`oxplow.turn`, `oxplow.task_effort`, `oxplow.nudge` (the producers with no prior
measure home); new dim `oxplow.token_kind`.

**Decision reversed — nudges are now IN the substrate (T-B, was tsk24).** The
earlier call kept the advisory nudges (report-less, coverage-target,
commit-hygiene, gauge-crossing) out of the substrate. T-B reverses it: with the
producer-spec layer in place, adding an `oxplow.nudge` event measure + one fact
per fired nudge is cheap and makes the `agent.nudges.fired` operational metric a
first-class spec like every other. The nudge rows in `agent_nudge` stay the
authoritative store; the fact is the analytics grain.

### Read surface (MCP)

`crates/oxplow-mcp/src/lib.rs`, agent-only in the surface-parity MANIFEST. The
measure-level primitives:
- `list_measures` / `list_dimensions` — the two catalogs (optional scope/
  subject_kind filter).
- `list_facts(measure_key, limit)` — raw atomic facts, most-recent, with the
  capture spine.
- `metric_series(measure_key, aggregation, group_by?, min_value?, severity?,
  stream?)` — the metrics-as-definitions read: one aggregated point per capture,
  optionally sliced by a dimension; `stream` scopes to one worktree's scans
  (like `metric_breakdown`).
- `metric_rollup(measure_key, dimension?)` — the by-dimension breakdown.

**The five metric-KEY reads are flipped onto the engine (T-C2, tsk35)** — they
resolve a `metric_spec` by key (seeded catalog) and compute over its
`source_measure` facts, no longer reading the legacy V38 `metric_sample`/
`metric_finding`/`metric_definition` store:
- `list_metric_definitions` → `fact_store.list_specs()` (the spec catalog; each
  row carries `source_measure` + `aggregation`, not a baked sample stream).
- `list_metric_samples(metric_key, limit, stream?)` → `series_for_spec`
  (newest-first, capped) — the metric-key ergonomic wrapper over
  `metric_series`; `stream` scopes to one worktree.
- `metric_breakdown(metric_key, dimension?)` → `rollup_for_spec` (default dim
  `oxplow.package`; an optional `stream` arg scopes to one worktree's scans
  (restored in tsk46) — facts aren't
  stream-partitioned at this grain).
- `get_metric_summary(metric_key, stream?)` → one `series_for_spec_in_stream`
  computation collapsed via `headline_from_series` (per the measure's temporal
  semantics) + the latest series point's captured_at/branch; `stream` scopes
  the headline to one worktree's timeline.
- `list_metric_findings(metric_key, capture_id?)` → `findings_for_spec` — the
  read-time offenders view (args changed from `run_id`).

**The IPC/Tauri counterparts are flipped too (T-C3a, tsk39)** — the four
`oxplow-rpc` cores + Tauri adapters now return spec/fact types
(`MetricSpec`/`SeriesPoint`/`RollupRow`/`FactFinding`), `bindings.ts` is
regenerated, and every metric frontend consumer moved with them (see the "Reads"
table below). Two measure-level IPC reads (`metric_series`/`metric_rollup`) were
added and their parity rows flipped `agent`→`both`. `SeriesPoint` gained
`git_version` + `source` (one-per-capture, like `branch`/`provenance`) so the
recordings table stays intact. Two frontend logic moves: the Explorer's group-by
is now a server `series_for_spec` `group_by` (each point carries `group`); the
`event`-kind subject breakdown is a server `rollup_for_spec("subject")`. The
`test`/`coverage` drill-ins fold into the uniform per-file/per-case `FactFinding`
table — the bespoke suite-tree / line-heat needed the verbatim legacy
`*-detail` payloads (pass/fail status, uncovered line-sets), which are **not in
facts** (the same scope-guard the coverage attribution took in T-D); the
underlying facts still chart + roll up.

**The baked writes are gone (T-C3b, tsk40)** — `record_baked_run` is deleted;
`run_one_gauge` is facts-only (returns the fact count; `record_gauge_facts`
emits `MetricSamplesChanged`). The 4 `plugins/metrics/code/*.star` scripts are
unbaked (emit only `{facts:[…]}`, no `tree:.`/`file:` samples). Any `samples`/
`findings` a script still returns (the per-language idiom scripts, not yet
unbaked) are computed-but-ignored. The legacy V38 `metric_sample`/`metric_finding`/
`metric_definition`/`metric_run` tables + `metric_store.rs` retire LAST in T-E
(tsk20).

### Catalog authoring surface (`measures:` / `dimensions:` config — workstream E)

Custom fact types and slice axes are **pluggable**, namespaced exactly like
metrics (`oxplow.*` reserved — those are the migration seed; config only *adds*
global/project entries). `crates/oxplow-config/src/lib.rs`:

```yaml
measures:                          # custom fact TYPES a collector may emit
  - key: acme.api_latency
    unit: ms
    subjectKind: endpoint
    temporalSemantics: non-additive   # additive | semi-additive | non-additive
    componentRole: numerator          # none | numerator | denominator
dimensions:                        # custom conformed slice axes
  - key: acme.license
    valueType: categorical            # categorical | numeric | temporal | entity-ref
    vocabulary: [MIT, Apache-2.0]     # optional controlled value set
    promote: true                     # request a generated column + index (see below)
```

- **Definition-only** (no `use:`/`key:` split like metrics — you declare a fact
  type / axis, you don't "enable" one). `validate_measures`/`validate_dimensions`
  mirror `validate_metrics` (namespacing, `oxplow.*` reserved, per-key
  uniqueness, enum checks). `resolve_measures`/`resolve_dimensions` merge the
  **global + project** scopes (project > global) into flat `Resolved*`.
- **Global scope** = `<global_config_dir>/{measures,dimensions}/*.yaml`
  (`load_global_measure_entries`/`load_global_dimension_entries`); auto-active in
  every project (unlike a global metric, which needs a project `use:`). The four
  `load_global_*_entries` loaders share one generic `load_global_entries` helper
  (tsk17 — they differ only in the doc field + validator).
- **Read-path caching (tsk17):** `resolved_specs`/`resolved_gauges` run on
  **every** snapshot event, so the four global YAML dirs are loaded once into a
  `MetricsService.global_catalog` (`Arc<Mutex<Option<GlobalCatalog>>>`) and
  served from cache (`with_global_catalog`); the cache is cleared on every in-app
  `ConfigChanged` (an external edit to a global file needs any in-app config op
  to refresh). Project config stays read fresh from the in-memory `RwLock`.
  `with_global_dir` forks a fresh cache (dir changed). Two more per-read memos:
  `effort_metric_deltas` loads each measure's history **once** across the
  File-family specs sharing it (a per-call `fact_cache`), and `dim_value` parses
  a fact's `dims_json` **once** per lookup (`parse_dims` + `dim_from_map`).
- **Boot seeding:** `MetricsService::seed_catalog()` runs once at boot and on
  every `ConfigChanged` (beside `seed_definitions`), upserting resolved
  measures/dimensions into the `measure`/`dimension` tables. `MetricsService`
  holds a `fact_store` via `.with_fact_store()`. Metric specs seed in two
  passes: the override-free built-ins (`builtin_metric_specs` /
  `builtin_ast_specs` / `builtin_producer_specs`) first, then EVERY resolved
  config spec — including a `use:` of a built-in, which resolves to scope
  `built-in` carrying the catalog default target plus the project's
  target/warnAt/failAt overrides (what `set_metric_override` writes). The
  second pass must not skip built-in scope, or those thresholds never reach
  the persisted `metric_spec` the engine reads.
- **Scaffolds:** `MetricsService::scaffold_measure` / `scaffold_dimension` —
  one-call "create a custom measure/dimension" (append config entry or write a
  shareable `<global>/…/<slug>.yaml`, reseed, return the key). The IPC/UI "New
  measure/dimension" buttons that surface these land with the UI task.
- **`promote`** now persists onto the row: `seed_catalog` threads the resolved
  dimension's `promote` into `NewDimension.promoted`, so `dimension.promoted`
  reflects the config (it was previously parsed but dropped at seed). Still
  **inert** downstream (see tsk28): the engine loads all facts and filters
  in-app, so the requested generated column + index bites nothing until reads
  go DB-side. Recorded, not yet acted on.

**Not yet done:** a **Dimensions catalog** UI page; `promote_dimension` teeth
(tsk28); unbaking the per-language idiom scripts (still emit dead `tree:.`/
`file:` samples, harmless — `run_one_gauge` ignores them); formula-spec wiring
(tsk21). Those are the open children of the epic. Already landed: the **MCP**
metric-key reads (T-C2), the **IPC + bindings + frontend** read surface (T-C3a,
tsk39), the **baked-write removal + 4-script unbake** (T-C3b, tsk40), the
**effort-attribution read** (T-D), and the **full V38 retirement** — the
capture is the run + detail envelopes (T-E1, tsk48), all legacy writes dropped
(T-E2, tsk49), tables + `metric_store.rs` dropped in V49 (T-E3, tsk50).

---


## Why it exists

`effort_observation` (see [collection.md](./collection.md)) was the first cut,
but it was **effort-scoped, CASCADE-deleted with its effort, and pruned to the
last 10** — so it couldn't answer "how did coverage move over the last month" or
"compare this branch to main." A coverage % or token count is a *datum about the
code/process at a point in time*, not a child of an effort. The substrate fixes
that: **time-primary, durable, dimension-sliceable**.

## The model (typed kinds, one fact table)

Don't force every measure into one blob row — the codebase's existing pattern is
*typed kinds with a uniform mechanism* (`CollectorKind`), and this extends it. A
small set of kinds (`gauge | findings | test | coverage | event`) share a common
envelope; each **projects ≥1 scalar sample** into the one narrow fact table the
explorer/feedback read; rich per-kind structure lives in typed detail.

Schema — `crates/oxplow-db/migrations/V38__metrics.sql`,
store `crates/oxplow-db/src/metric_store.rs` (`SqliteMetricStore`):

- **`metric_definition`** — the catalog. `key` (namespaced; `oxplow.*`
  reserved), `kind`, `title`/`unit`, `direction` (higher/lower/neutral),
  `default_agg`, `grain`, `basis`, `producer`, `description`/`category`/
  `language`, `scope` (built-in|global|project), `dimensions_json`,
  `target`/`warn_at`/`fail_at`. Upserted by `key`.
- **`metric_dimension`** — conformed-dimension catalog (seeded: time, stream,
  thread, effort, git_version, branch, subject, model, agent, language,
  severity, status). Shared meaning across metrics → cross-metric drill-across.
- **`metric_subject`** — subject hierarchy (file→module→package→repo) for
  roll-ups. Declared, not yet exercised.
- **`metric_run`** — a compute event (generalizes `code_quality_scan`): producer,
  status, trigger, provenance/source, snapshot/git/**branch**. One run can feed
  many metrics. Raw events have **no run** (`run_id` NULL).
- **`metric_sample`** — the durable scalar fact (the BI grain). `value`
  (+ `numerator`/`denominator` for ratios so roll-ups RE-AGGREGATE correctly),
  `captured_at` (the spine) + `closest_git_version` + `branch`, optional
  `subject_kind`/`subject_ref`/`path`/`line`, `dims_json`, `provenance`/`source`.
- **`metric_finding`** — located detail for the `findings` kind (generalizes
  `code_quality_finding`): path/line, kind, severity, rule, message, value.

### Time-primary, effort-as-overlay (the key invariant)

A sample carries **NO `effort_id` FK**. It's anchored by `captured_at` +
`closest_git_version`. Efforts (and later commits/releases) are **time-range
overlays** read from `task_effort` (`started_at`/`ended_at`) — so:
- efforts can be garbage-collected without touching a single sample,
- a sample can fall in zero or many efforts,
- a `diff-vs-effort-start` metric stays interpretable via its `basis_ref`
  baseline version after its effort is gone.

"Group by effort" = bucket samples whose `captured_at` ∈ the effort's window
(`SqliteMetricStore::samples_for_effort`). No count-prune, no CASCADE; optional
age sweep only.

### Effort attribution — spec-driven, over facts (T-D, tsk36)

`CollectionService::effort_metric_deltas` reads the **spec catalog**
(`list_specs()`) and, per **metric family**, aggregates the effort's own **facts**
(no legacy sample read; the `EffortMetricDelta` DTO shape is preserved, so the IPC
+ `EffortMetrics.tsx` are untouched). Two capture-resolution spines back the four
families:

- **claimed files × time** — code gauges are snapshot scans, so their captures are
  **not** effort-stamped; the File family reads them by claimed path + capture time,
  scoped to the effort's stream. Exception: an `on-effort-complete` gauge run KNOWS
  its producing effort and stamps the capture (tsk43 — `GaugeRunContext.effort_id`),
  so its just-after-close capture still counts as the effort's "current".
- **`metric_capture.effort_id`** — the run/operational producers stamp the owning
  effort at ingest (tsk37, `resolve_owning_effort`), so an effort's run + token +
  nudge facts are exactly `captures_for_effort(effort_id)` → `facts_for_captures`.

| family | how the delta is computed |
|---|---|
| **File** — snapshot-scan gauge (`display_kind` ∈ {`gauge`, `findings`}, a source measure, no formula, non-producer, non-operational — includes the `static-quality` built-in code gauges, whose captures are never effort-stamped; tsk43) | Σ over the effort's **claimed files** (`task_effort_file`) of `(current − baseline)`, each fact contributing per the spec's **aggregation** (`count` ⇒ 1 per offender — matching the Metrics page — else the fact value); facts are scoped to the effort's **stream** (worktree). Baseline capture = latest before the effort start; current = latest at/before the effort end (newest when open; a capture STAMPED with this effort — an on-effort-complete gauge run — also counts). A CLOSED effort with no in-window capture yields no row (never a post-close capture, never a fabricated drop-to-zero). A claimed file absent from a capture = 0 (sparse emission → a drop-to-zero is seen), and the producers' EMPTY zero-hit captures are spliced into the timeline so a scan that found nothing is eligible as baseline/current (tsk44). **No claims, or repo-scalar facts with no path** → the repo-wide before→after fallback. `file_delta_from_facts` |
| **Run** — tests (category `testing`) + the `oxplow.analysis.*` producer pair | before→after (or `sum` flow) over `aggregate_series` of the facts of the effort's OWN captures (`facts_for_captures(measure, captures_for_effort)`). Analysis is classified Run via the producer-key check (its facts arrive on effort-stamped run-ingest captures), so it never reaches the File branch (the tsk272 guard) |
| **Window** — operational (`agent.*`/`effort.*`/`task.*`) + formula/event specs | identical read to Run now that captures carry `effort_id`; kept a distinct family only to document it has no run-claim write side. `effort_stamped_delta` serves both |
| **Coverage** (category `coverage`) | effort-relative: for each coverage run CAPTURE this effort **claimed** (ledger — the capture is the run, T-E1), `coverage_delta_for_spec` derives the **diff-coverage** at read (`diff_coverage_for_effort`) from the capture's ABSOLUTE per-file **line-sets** (`metric_capture.detail_json`, the `coverage-detail` envelope), then before→after over the derived sequence. The coverage FACTS carry num/den counts; the line-sets live only in the detail envelope |

The family is chosen by **one classifier** — `classify_effort_attribution(spec)
→ EffortAttributionFamily` (`crates/oxplow-app/src/attribution.rs`, beside the
write-side `AttributionKind` each maps to: File↔`FileKind`, Coverage/Run↔`RunKind`,
Window↔no-claim). `effort_metric_deltas` `match`es on it; adding a fact-kind is one
variant + one match arm, not a scattered if/else chain (tsk274). A formula spec (no
source measure) falls through to Window and no-ops.

The ledger-run-claim ∪ (the `capture.effort_id` spine) is the intended end state;
T-D lands on the stamped spine alone (the common auto-attributed case). A run
CLAIMED post-hoc (`claim_runs` at close) whose capture wasn't stamped at ingest is
the deferred backfill (tsk38). The now-orphaned legacy reads
(`file_samples_for_paths`, `samples_for_effort`, `samples_for_runs`,
`list_findings`, `runs_in_window_by_trigger`) are swept in T-E3 (tsk20).

### Run attribution grain — the ledger, not the clock (tsk260/tsk269)

**The capture IS the run (T-E1, tsk48).** Agent-work runs — tests, coverage,
analysis — are **observe-always**: every run writes its `metric_capture` + facts
regardless of how many efforts are open, attributed through the `capture.effort_id` stamp
(T-D read) + the `effort_attribution` ledger (the write/reconcile side), never by
time window — because parallel sub-agents in one thread run different runs
concurrently and the clock can't tell them apart. All stamp `trigger='on-report'`,
and each carries its verbatim payload in `metric_capture.detail_json` as the
envelope `{"kind": "test-detail"|"coverage-detail"|"analysis-detail", "payload":
{…}}`. At record time the producer resolves the owning effort — when the caller
named a `task_id` (exact) or exactly one effort is open — stamps
`capture.effort_id`, and writes a `claimed` ledger row for **`run:<capture_id>`**;
the concurrent-unnamed case is left for the agent to claim at close (`claim_runs`
on `complete_task`/`update_task`/`amend_effort` — the ids in those refs are
capture ids now). `RunKind` OBSERVES via `captures_in_window_by_trigger`; the
EFFORT REVIEW's `describe_run` reads the claimed capture + its envelope. The
`effort_observations_from_metrics` read joins the ledger (claimed capture ids →
`get_capture` → the detail envelope); the metric-delta read (above) joins
`capture.effort_id`. **Coverage** is effort-relative (diff vs the effort's start
snapshot), so it observes the ABSOLUTE report always and DERIVES the effort diff
at read (`diff_coverage_for_effort` over the capture's `coverage-detail`
envelope) — a run claimed after close still yields a diff (tsk270). The mechanic
+ trait (`AttributionKind`/`RunKind`) live in `.context/agent-model.md` +
`.context/data-model.md`.

### Additivity

Ratio metrics (coverage %, pass rate) store `numerator`+`denominator`. Roll-ups
MUST re-aggregate from components (`aggregate_ratio` = Σnum/Σden), never naive-
AVG a percentage. Non-ratio gauges use `default_agg`.

## Branch tracking

Runs and samples record the **branch** they were captured on (`branch` column,
a conformed dimension), when applicable (NULL for detached HEAD / non-git /
operational metrics). Captured via `oxplow_git::detect_current_branch` in the
code-fact producers; operational producers (tokens) leave it NULL.

## Producers (how samples get written)

Producers are the only thing that writes samples. They're best-effort
side-bands on the host path (a metric write error is logged via `tracing::warn!`,
never fails the host path). For coverage/tests/analysis the substrate is now the
**sole** store (the legacy `effort_observation` table was dropped, tsk215) — the
mirror helpers also write a verbatim `*-detail` `metric_finding` (test
suite/case tree, coverage per-file uncovered lines, analysis payload) so the
panel can reconstruct full detail via `effort_observations_from_metrics`:

| producer | where | emits |
|---|---|---|
| coverage / tests / analysis | `crates/oxplow-app/src/collection.rs` (`mirror_coverage_metric` / `mirror_test_metrics` / `mirror_analysis_metrics`, called from `observe_coverage`/`record_test_run`/`record_static_analysis`) | `oxplow.coverage.abs_pct` (absolute; diff derived at read); `oxplow.tests.{passed,failed,total}`; `oxplow.analysis.{errors,warnings}` + a finding per lint hit + a `*-detail` finding carrying the verbatim payload |
| otel-tokens | `crates/oxplow-app/src/token_usage.rs` (`ingest_otlp_tokens`, fed by the control-plane OTLP receiver — tsk22) | per-model `agent.tokens.{input,output,total}` from Claude's `claude_code.token.usage` OTEL counter. Tokens only — no derived USD cost (rates move; a stale price table is worse than none). The transcript `on_stop` path now projects only `agent.turns` + the per-turn `agent_token_usage` prompt rows |
| effort-lifecycle | `crates/oxplow-app/src/task_service.rs` (`project_effort_lifecycle_metrics`, called when `update()` closes an effort on an `in_progress` exit) | derived `effort.cycle_time_ms` (close − start, subject=effort) + `task.efforts` (efforts-so-far, the redo-rate signal) from `task_effort`; branch captured when the stream has a worktree |
| nudges | `crates/oxplow-app/src/collection.rs` (`project_nudge_metric`, called from `persist_nudge` after a fired nudge records) | `agent.nudges.fired` (event kind, run-less; value 1, subject=the nudge `kind`) — an agent-activity signal |
| config gauges | `crates/oxplow-app/src/metrics_service.rs` (`MetricsService`) — the author-able runner. Seeds a `metric_spec` per resolved `metrics:` entry (+ a legacy `metric_definition` until the read-flip); runs each **gauge** (`resolved_gauges()` = config `gauges:` ∪ `use:`-enabled built-ins) on its trigger (`on-snapshot` via the snapshot-batch event in `run()`; `on-effort-complete` via the `task_service.rs` ride-along; `manual` via `run_metric_by_key`) | one `fact` per `GaugeFact` the script emits (bound to a defined measure in the gauge's `emits`), version/branch/snapshot-stamped, under one `metric_capture`. Facts-only (T-C3b): `run_one_gauge` writes nothing but facts; any `samples`/`findings` a script still returns are ignored |

> Navigation / activity (`page_visit`, `usage_event`) are **deliberately not
> projected** into the substrate: they're oxplow-usage telemetry (UI metadata),
> not code or agent-activity metrics, so they stay in their own tables.

Each producer: `upsert_definition` (idempotent) → `record_run` → `record_sample`(s)
→ emit `OxplowEvent::MetricSamplesChanged { stream_id }`.

> **The plan is for these to become bundled plugins** (jaq/Starlark/exec,
> registered via `with_builtins()`) so producers are *content*, not hardcoded
> Rust (tsk218). The hardcoded mirror helpers are the interim. The legacy
> `effort_observation` path has already been **dropped** (tsk215) — the substrate
> is the sole store.

> **Producer-metric registry (tsk286/tsk287).** Because these producers only
> `upsert` their definition at *record* time, the Catalog (a registry of
> *available* metrics) can't discover them before first data. So the canonical
> always-on producer metrics live in **`producer_metrics.rs`**
> (`builtin_producer_metrics()` + `ProducerMetric::definition()`) as the **single
> source of truth**: the producers build their `NewMetricDefinition` via
> `producer_metric(key).definition()` (no inline descriptors), and `catalog()`
> unions the same list. Add/rename a producer metric in **one** place. Coverage's
> red/green thresholds (`target`/`warn_at`/`fail_at`) are policy applied by the
> coverage producer on top of the registry descriptor, not part of the registry.

## Read surface

> **Flipped (T-C2, tsk35 + T-C3a, tsk39):** the metric-key reads on BOTH the MCP
> and the IPC surface now read the fact substrate via `MetricEngine`
> spec-wrappers, NOT this V38 store. The paragraph below is the historical V38
> shape, kept for context; the current IPC wiring + types are in the **IPC**
> bullet.

- **MCP** (`crates/oxplow-mcp/src/lib.rs`): reads `list_metric_definitions`
  (optional language/scope filter), `list_metric_samples` (by key, newest-first),
  `list_metric_findings` (by run id — findings-kind drill-in),
  `get_metric_summary` (latest value + delta-vs-target), and `metric_breakdown`
  (tsk327/330 — rolls a per-file metric up by a **dimension** via
  `SqliteMetricStore::dimension_rollup_for_metric`: the latest
  `subject_kind='file'` sample per file, summed by `dimension` key, largest
  first — the dormant `metric_subject` package grain made concrete; "which
  package / language holds the most complexity/TODOs"). `dimension` is
  `"package"` (default — parent dir) or any per-file `dims_json` key (e.g.
  `language`, tsk319); `stream` is optional (omit ⇒ all streams), matching the
  UI. The same store method backs the IPC
  `metric_dimension_rollup` + the Metric Detail **Breakdown** card (tsk328/319). Authoring/trigger:
  `run_metric` (run a configured gauge now — the `manual` trigger → `MetricsService::run_metric_by_key`;
  returns `facts_recorded`) and `record_metric` (an **asserted FACT** on the
  metric's source measure, under a `provenance: asserted` / `source:
  agent-reported` / `scan_kind: asserted` capture **anchored to the stream's
  latest snapshot for provenance** (tsk71/tsk72 — the snapshot says which tree
  state the value described; the `asserted` scan kind keeps it from being read
  as a scanned set) — flipped off the legacy sample write in tsk41 so the
  fact-based reads actually see it; a formula spec is rejected, and so is a
  `count` spec — one asserted fact would read as 1 whatever its value. The fact
  is stamped to match the spec's own filter (severity / dim_eq → the `rule`
  column for `oxplow.rule`, dims_json otherwise) and carries ratio components
  for a `ratio` spec (den=100 for `%` so the percent round-trips), so the
  metric's own reads actually include the asserted number). These four are
  **agent-only** (classified in the surface-parity manifest); the renderer
  drives compute via config + the runner, not ad-hoc IPC.
- **IPC** (`crates/oxplow-rpc/src/commands/metrics.rs` cores +
  `crates/oxplow-tauri-ipc/src/commands/metrics.rs` Tauri adapters, registered
  in `collect_commands!` + the remote `rpc_dispatch!`) — **flipped onto specs +
  facts (T-C3a, tsk39)**, mirroring the MCP wiring: `list_metric_definitions` →
  `list_specs` (`MetricSpec`), `list_metric_samples(metric_key, limit, group_by?)`
  → `series_for_spec` (`SeriesPoint`, newest-first; `group_by` slices server-side),
  `metric_dimension_rollup(metric_key, dimension)` → `rollup_for_spec` (`RollupRow`,
  also serves the `event`-kind `subject` breakdown), `list_metric_findings(metric_key,
  capture_id?)` → `findings_for_spec` (`FactFinding`, `both`-scoped — the per-capture
  drill-in, args changed from `run_id`). Two measure-level reads `metric_series` /
  `metric_rollup` are `both`-scoped mirrors of the MCP tools. Bindings regenerate
  to `MetricSpec`/`SeriesPoint`/`RollupRow`/`FactFinding`. `list_effort_metric_deltas`
  (tsk250, `ui`-scoped — `commands/effort.rs`) returns the family-attributed
  per-effort roll-up (`EffortMetricDelta`) for the task-page panel; the agent
  gets the same numbers as prompt text via `effort_metric_context`.
- **Event**: `OxplowEvent::MetricSamplesChanged { stream_id }` (coarse — the
  renderer refetches).

## UI

The metric UI is **four separate pages**, each with one job, all reading the one
fact table — no per-metric UI code. Each is registered like the `usage` index
page (`tabState.PageKind`, `pageRefs.indexRef`, `RailHud/sections.ts`, `App.tsx`,
`pageKinds.tsx`) and cross-links to the others in its header. The split happened
in two steps: the Catalog off first (tsk282), then Explorer/Recorded/Detail
(tsk283). The three **observe** pages are read-only; the Catalog is the only one
that **writes**.

- **Metrics Explorer** (`MetricsExplorerPage.tsx` wrapping `MetricsExplorer.tsx`)
  — the marquee page and the rail "Metrics" entry (`indexRef("metrics")` /
  `metricsExplorerRef()`). Header links: "Recorded metrics →", "Configure
  metrics →". A measure's title navigates to the metric's **detail page** (via
  `onOpenDetail` → `metricRef`).
> ### ⚠️ A seeded spec does NOT mean an enabled metric (tsk87)
>
> `seed_catalog` seeds **every** built-in spec (`builtin_metric_specs` /
> `builtin_ast_specs` / `builtin_producer_specs`) unless a config `enabled: false`
> marker explicitly prunes it. A built-in gauge that is merely **un-`use:`d keeps
> its spec** — it just never RUNS (`resolved_gauges` elides it). But `catalog()`
> computes a built-in gauge's `enabled` as *"a non-disabled `use:` resolves it"*.
>
> So `metric_spec` ⊋ "the enabled set", and **only the catalog knows about
> `use:`**. Reading `list_metric_definitions` alone and calling the result
> "enabled metrics" is wrong: in this Rust/TS repo the bundled `oxplow.csharp.*`
> and `oxplow.clojure.*` idiom specs are seeded, never run, and have no facts —
> so Recorded Metrics listed them as permanent `—` rows while Metric Settings
> showed the same rows *unchecked*. That's why the page's row set is the
> **catalog**, with the spec joined in by key for presentation metadata.
>
> (The "spec table = the enabled set" phrasing under the collection gate above is
> about the **producer** measures, where disabling does prune. Don't generalize it
> to gauges.)
>
> Note enabling a C# gauge here still wouldn't show `0`: `oxplow.ast_hit` is
> `capture_scope: per-path`, whose zero-fill is deliberately suppressed, so a scan
> that matches no files yields no point at all. Nothing auto-detects a project's
> languages.

- **Recorded Metrics** (`RecordedMetricsPage.tsx`, `PageKind`
  `"metrics-recorded"` / `recordedMetricsRef()`) — every **catalogued** metric as
  a `title · trend sparkline · latest value` row (row set = `list_metric_catalog`,
  the only source that knows `use:`; the seeded spec joins in by key for unit /
  direction / thresholds and is null only for an explicitly-disabled metric whose
  spec was pruned). The rail's **Show** dropdown picks `Enabled` (**default**) /
  `All` — see the box above for why that distinction isn't free. Pure row
  filtering (Show mode + search, composed so a search never resurfaces a disabled
  metric) lives in `recordedMetricsRows.ts`. A section only renders when it has
  rows, since `buildMetricSections` groups what it's given; a filtered-empty list
  falls back to a "No metrics match" state. Rows are colored by `statusColor`
  (target/`fail_at`/direction). The value sits **after** the sparkline (tsk82)
  because it *is* that sparkline's last point — both read the same
  range+branch-filtered `samples` (newest-first, so `samples[0]`), meaning the
  "latest value" is the latest **within the selected filters**, not all-time. Each `<tr>` adopts browser-style
  click via `useRouteDispatch(metricRef(key))` (plain-click → detail in-tab,
  modifier/middle/right → new tab). Header links: "Explorer →", "Configure
  metrics →". Live-refreshes on `metricSamplesChanged`.

> ### Sectioning — one rule, both pages (`buildMetricSections`, tsk81)
>
> Recorded Metrics renders those sections through the shared
> `CollapsibleSections` / `CollapsibleSection` primitive (tsk84) — a chevron on
> each section header, with the **Expand all / Collapse all pair living in the
> details rail** beside the filters (`SectionCollapseControls`, tsk86). Collapsed
> state persists under `pageKey: "metrics-recorded"`. The provider wraps the whole
> `<Page>` so its context reaches the rail as well as the body. See
> `.context/usability.md` → "Collapsible page sections". Metric Settings has not
> adopted it (its headers already carry the tri-state group checkbox).
>
> Recorded Metrics and Metric Settings render the **same section list**, built by
> the shared pure `buildMetricSections(rows, getCategory, getLanguage)` in
> `metricCategories.ts`: categories in `CATEGORY_ORDER`, **except
> `static-quality`**, which gets no section of its own — its real top-level
> division is by language, so each language is promoted to a top-level section
> (a peer of Tests / Coverage / Operational) and the language-agnostic analysers
> (`oxplow.analysis.*`) fall under **"General"**. Both pages call the one helper;
> the rule is deliberately **not** restated per page, because two copies drift
> into two different groupings of the same metrics.
>
> **Both pages now group off `MetricCatalogEntry.language`** — Metric Settings
> always did; Recorded Metrics joined it when its row set became the catalog
> (tsk87). For a built-in gauge the catalog takes that slug straight from the
> *gauge* (`builtin_metrics()`), so the two agree by construction.
>
> `builtin_ast_specs` nevertheless **reads each spec's language off its gauge by
> key** rather than restating the slug
> (`builtin_ast_specs_carry_the_language_their_gauge_declares` pins it). Before
> tsk81 the specs set no `language` at all (`NewMetricSpec::base` defaults it to
> `None`). That's no longer what sections the *page* — but `MetricSpec.language`
> is still real read surface: `list_metric_definitions` takes a **language
> filter** over it (IPC + MCP), which silently matches nothing when the column is
> null. Keep it populated.
>
> Note the key segment is **not** the slug: `oxplow.ts.*` is language
> `typescript`. A gauge's `language: ""` (the language-agnostic code gauges) maps
> to spec `None` — `""` is not a language, and `groupByLanguage` reads null/`""`
> as its "General" bucket.
- **Metric Detail** (`MetricDetailPage.tsx` wrapping `MetricDetail.tsx`,
  `PageKind` `"metric-detail"`, routed by `metricRef(key, effort)`) — its own
  page (tsk283), navigated into from the Explorer, Recorded Metrics, the
  **Catalog** (each metric name is a `RouteLink` to `metricRef(key)`, tsk33), and
  the task-page EffortMetrics drill-in (so there's no inline overlay). Back goes
  through `PageNavigationContext` (`goBack`, falling back to Recorded Metrics).
  The metric name is the H1; the definition's **`description`** renders as intro
  text under it (tsk309). Layout is the details layout: a right rail ("Details")
  holds the range/chart-mode/branch controls + the agg-aware in-range stat + the
  full **definition metadata** (`MetricStatsRail`, tsk33: ID/key, Type
  (display_kind), Aggregation, source Measure, Scope, Category, Language, Unit,
  Direction, Target, Warn/Fail thresholds, Branch); the main column has the trend
  chart → **paginated** recordings table (`RecordingsTable`, 25/page) → kind
  drill-in. See the `MetricDetail` component bullet below for what each kind
  renders.

> **Definition descriptions (tsk309).** Every metric carries a one-line
> `description` (on `metric_definition`). It's inherent to the definition — set
> once and not overridable by a `use:` entry (`resolve_one` reads `def.description`,
> like trigger). Sources: the built-in code gauges (`BuiltinMetric.description`),
> the always-on producers (`ProducerMetric.description` in `producer_metrics.rs`),
> and config `key:` entries (`MetricEntry.description` → `ResolvedSpec` →
> `spec_definition()`).

`MetricsExplorer.tsx` itself (the chart component, P4) is a multi-measure
overlay on one time
  axis + group-by a conformed dimension (`branch`/`subject`/declared dims like
  `model`/`language`) + **line / bar / scatter** viz + target band + legend.
  Inline SVG (no charting lib, like the codebase's other visuals); pure grouping
  in `buildExplorerSeries`, pure pairing in `buildScatterPoints` (two measures ×
  a shared group → one point per group value, e.g. coverage × complexity by
  module). **Saved views** (`metricsPresets.ts`, localStorage): name the current
  measures/group-by/viz and reload it later; the picker also offers **built-in
  presets** (`BUILTIN_PRESETS`: "Tokens by model", "Coverage", "Tests pass/fail")
  and the page accepts an `initialPreset` deep-link. **Effort bands** (tsk233):
  the efforts overlapping the charted window (`list_efforts_in_window`) render as
  faint bands behind the series — hover names the effort, click scopes the chart
  to that window (Clear resets). A measure's title links to its per-kind
  **detail** (via `onOpenDetail`).

- **Metric Recording** (`MetricRecordingPage.tsx`, `PageKind` `"metric-recording"`,
  `metricRecordingRef(runId, {metricKey,capturedAt,value})`, tsk313) — drill-in
  from a **single recording**. The Recordings-table rows on the Metric Detail
  page are clickable when the sample has a `run_id`; clicking opens this page,
  which lists the run's **`metric_finding`s** (`list_metric_findings(runId)`) —
  the located items the gauge counted (file:line · name · value). This is how a
  "count of X" gauge (high-complexity functions, long functions) becomes
  drillable: the gauge **emits findings** alongside its samples (see the gauge
  findings channel below). Degrades to an empty state when a run has no findings.

`MetricDetail.tsx` (+ pure `metricDetailData.ts`, tsk232) is the renderer
`MetricDetailPage` mounts: one view selected from `metric_definition.kind`.
Every kind shows the value trend (+ Δ-vs-first, branch, trust badge); each adds
its drill-in from the latest run's findings (`list_metric_findings`):
**findings** → a findings table, **test** → the suite/case tree (from the
`test-detail` payload), **coverage** → per-file uncovered changed lines
(`coverage-detail`), **event** → top-N subjects, **gauge** → trend only.

The **configure** page (tsk282):

- **Metric Settings** (`MetricsCatalogPage.tsx` wrapping `MetricsCatalog.tsx`,
  P4) — a dedicated top-level page (`PageKind` `"metrics-catalog"`,
  `metricsCatalogRef()`, launcher Activity category), the only metrics surface
  that **writes**. **Titled "Metric Settings" since tsk80** — "Catalog" read as a
  browsable index, which is the *Recorded Metrics* job; this page is where you
  configure. The `metrics-catalog` **slug is deliberately unchanged** (page kind,
  tab id, `metricsCatalogRef()`, `page-metrics-catalog` testid) so existing refs,
  bookmarks, and probes keep resolving — only the user-visible label moved.
  It's a **registry of everything available**, NOT a list of metrics with
  recorded data — every metric the system can produce is listed via
  `list_metric_catalog`, **grouped by category** (Code gauges / Tests / Coverage
  / Static analysis / Operational), even before any sample exists. `catalog()`
  unions **four** sources, deduped by key: (1) the bundled code gauges
  (`builtin_metrics()`, toggleable); (2) project/global `metrics:` entries
  (toggleable); (3) the built-in always-on producers
  (`builtin_producer_metrics()` — tokens, tests, coverage, analysis, effort
  lifecycle, nudges — listed regardless of recorded data so the user sees they
  exist, tsk286); (4) every other seeded `metric_definition` — installed plugin
  metrics and legacy rows. **Every entry is `toggleable: true` (tsk31)** — the
  "always on" class is retired: producers/plugins can be enabled/disabled just
  like code gauges. `catalog()` reads each row's `enabled` from config
  (`config_state`): a built-in gauge is on only when a non-disabled `use:`
  resolves it; producers/plugins are default-ON unless an `enabled: false` marker
  disables them. **Layout (tsk29):** each category is a *section*
  (`<h2>` title + a list of metric rows), NOT one flat table — a row shows only
  the on/off **checkbox**, the metric **name** (a `RouteLink` to its Metric
  Detail page, tsk33), and its **target** (no kind / scope / trigger / raw key
  columns; the key rides as a hover `title`). Each
  section header carries a **tri-state group checkbox** to the right of the title
  (`GroupCheckbox` + pure `sectionCheckboxState`, tsk32): checked when every
  metric is on, indeterminate when only some are; a click enables all (from
  off/indeterminate) or disables all (from fully-on) in one batch write
  (`set_metrics_enabled` — one config write + one reseed for the whole section).
  **Static analysis
  has no single section** — its real top-level division is by language, so each
  language becomes its own top-level `<h2>` section (peer to Tests / Coverage /
  Operational), ordered via `groupByLanguage` in `metricCategories.ts`: the
  language-agnostic code gauges + analysis producers fall under **"General"**
  (first), then Rust / TypeScript / C# / Clojure / … by display label. (The
  `groupCatalog` category still positions the whole static-analysis block where
  `static-quality` sits in `CATEGORY_ORDER`; the render expands that one group
  into N per-language sections instead of an umbrella.)
  Enable/disable via `set_metric_enabled` — its config shape is default-aware
  (`apply_metric_enabled` + `is_default_on`): a default-OFF metric (built-in code
  gauge / global def) toggles by the presence of a bare `use:` entry, while a
  default-ON metric (producer/plugin) or a config `key:` definition toggles by an
  `enabled: false` **marker** (so disabling never deletes a `key:` definition).
  `seed_catalog` then **reconciles** the `metric_spec` table down to exactly the
  enabled set — upsert the enabled, `delete_spec` the disabled — so all
  spec-driven reads (Explorer/Recorded/Detail/effort-deltas/MCP) go empty for a
  disabled metric, and its producer's collection stops via the
  `measure_has_active_spec` gate (see the producer section: **base data is not
  collected when no active metric consumes its measure** — shared-measure
  families like `oxplow.tokens`/`oxplow.test_case` keep flowing until *all* their
  metrics are off). Historical facts are never deleted, so re-enabling restores
  the chart. **Inline-edit the target** (tsk233) via
  `set_metric_override` → `MetricsService::set_metric_override` writes the
  target override onto the `use:` entry. **Trigger is inherent to the
  definition** — *when* a metric is collected is a property of what it measures,
  not a per-project knob — so it's shown **read-only** and never user-pickable;
  `resolve_one` reads it from the definition (like `compute`), a `use:` entry
  can't override it, and `set_metric_override` no longer accepts it (tsk290).
  **"New metric"** scaffolds the **trio** (measure + gauge + metric) at
  **project** or **global** scope: `scaffold_metric` →
  `MetricsService::scaffold_metric` writes a starter fact-emitting Starlark stub +
  a `measures:` entry (`<key>.count`) + a `gauges:` entry (`<key>`) + a `metrics:`
  spec (`<key>`, `sum` over the measure). *Project* writes the script under
  `oxplow/gauges/<slug>.star` + the three entries in `.oxplow/project.yaml`,
  returns the project-relative path, and the page opens it. *Global* writes the
  script + three manifests under `<global_config_dir>/{gauges,measures,metrics}/`
  (via `write_global_{gauges,measures,metrics}_file`) **and** adds a project `use:`
  so the metric is active here (the global gauge + measure are active
  automatically; a global metric is library content until a project opts in). The
  runner resolves each gauge's `entryFile` against the right base dir
  (`script_base_dir`: `<global>/gauges` for a global-scope gauge, else the project
  dir).

Metrics are also surfaced **organically off the Metrics pages** (tsk250): the
task/effort page renders an `EffortMetricsBlock` (`components/EffortMetrics.tsx`)
under each effort — the metrics whose facts the effort touched, as compact
before→after rows **grouped by type** (`metricGroup`), self-hiding when empty and
live on `metricSamplesChanged`. A row drills into the metric's detail via
`metricRef(key, {effortId,start,end})` → `MetricDetailPage` (`metricKey` /
`effort` props) renders `MetricDetail` with an **"In this effort"**
before→after callout (+ per-file count) above the full trend.

Catalog reads/writes: `list_metric_catalog` + `set_metric_enabled` +
`set_metric_override` (RPC cores in `commands/metrics.rs`, Tauri adapters,
`ui`-scoped in surface-parity), backed by `MetricsService::{catalog,
set_metric_enabled, set_metric_override}` and the `MetricCatalogEntry` type.
**Token Analytics is retired** as a bespoke page (tsk233) — its `token-analytics`
tab now renders the **Metrics Explorer** page with the "Tokens by model" preset.
(Page/Usage
analytics stay bespoke: `page_visit`/`usage_event` are deliberately **not**
projected into the substrate — see the producers note above.)

## Adding a metric (today)

1. Pick a namespaced `key` and a kind.
2. In the relevant producer, `upsert_definition` it and `record_sample` with the
   value (+ components for ratios), subject, and dims.
3. It then appears in MCP/IPC reads and on the Metrics page automatically — no
   UI code per metric.

## Authoring surface (the four config blocks — epic tsk12, E)

A project (or the user-global library) declares metrics in YAML — no Rust per
metric. The substrate is dimensional, so authoring splits into **four orthogonal
blocks** (matching the real cardinality): `measures:` (fact TYPEs) ← `gauges:`
(fact PRODUCERs) → facts → `metrics:` (read SPECs), sliced by `dimensions:`.
Parsed/validated/resolved in `crates/oxplow-config/src/lib.rs`
(`MetricEntry`→`ResolvedSpec` + `resolve_metrics`; `GaugeEntry`→`ResolvedGauge` +
`resolve_gauges`; `GaugeComputeConfig`; `load_global_{metric,gauge}_entries`); the
runner (`MetricsService`) seeds a `metric_spec` per resolved metric (and a legacy
`metric_definition` until the read-flip, tsk26) and runs each **gauge** on its
`trigger`.

```yaml
measures:                             # the fact TYPE the gauge emits
  - key: repo.todo_count
    subjectKind: file
    unit: count
    temporalSemantics: semi-additive  # additivity OVER TIME
gauges:                               # the PRODUCER (runs a script, emits facts)
  - key: repo.todo
    trigger: on-snapshot              # on-report|on-snapshot|on-effort-complete|manual|continuous
    emits: [repo.todo_count]          # declare-to-collect allow-list
    compute: { runtime: starlark, entryFile: oxplow/gauges/todo.star }
metrics:                              # the read SPEC (the chartable metric)
  - key: repo.todo_count              # DEFINE — a measure aggregation
    sourceMeasure: repo.todo_count
    aggregation: sum                  # count|sum|avg|min|max|last|ratio (within a capture)
    direction: lower-better
    unit: count
    displayKind: gauge                # gauge|findings|test|coverage|event
    filter: { minValue: 1 }           # optional predicate before aggregating
    sliceableDims: [language]
  - use: myglobal.todo_density        # ENABLE a catalog metric (+ threshold overrides)
    target: 5
```

- A **metric** no longer computes anything — it's a pure spec (`sourceMeasure` +
  `aggregation` + optional `filter`, OR a `formula: {op,left,right}` over other
  metrics). Two-axis aggregation: `aggregation` combines facts *within a capture*;
  the source measure's `temporalSemantics` governs the cross-time collapse. A
  `use:` may only re-target thresholds; the structural fields are inherent.
- The **gauge** script returns `{ "facts": [ {measure, value, subject?, path?,
  line?, rule?, dims?} ] }` — one atomic fact per subject (never a baked total),
  each on a measure in the gauge's `emits`, calling the `files(glob)` /
  `ast_query(text, language, sexpr)` host builtins (see [collection.md](./collection.md)).
  A gauge may also return `"samples"`/`"findings"` (`GaugeFinding`, tsk311) —
  the legacy baked channel, kept for the built-in gauges until the read-flip; a
  facts-only gauge (the clean model) writes no baked run.
- **Three scopes**, precedence **project > global > built-in** by key:
  - **built-in** — the bundled catalog
    (`oxplow_collect_plugin::builtin_metrics()`; scripts under
    `crates/oxplow-collect-plugin/src/plugins/metrics/<lang>/`, embedded via
    `include_str!` in `builtin_metrics.rs`). Each authored through the **public**
    surface (`files()`/`ast_query()`) — no privileged Rust path — and verified by
    a golden test over a fixture corpus. A project activates one with
    `metrics: - use: oxplow.<lang>.<name>`; the runner builds the collector from
    the embedded script (`BuiltinMetric::collector()`), never a project-disk
    file. Two families:
    - **Language-agnostic code metrics** (tsk314) — one metric, all languages —
      `oxplow.todos`, `oxplow.fn_count`, `oxplow.high_complexity_fns`,
      `oxplow.long_functions`. Built via the `code_gauge` helper with
      `language: ""`; the scripts (under `plugins/metrics/code/`) sweep the
      `source_files()` reader and call a capability (`code_metrics()` /
      `markers()`), so the per-language knowledge lives in `oxplow-code-metrics`,
      not the metric. See "Language-agnostic capability layer" below.
      `source_files()` **excludes codegen output** (tsk68) via
      `oxplow_code_metrics::is_generated_source` — a `generated` path segment,
      a `.generated.`/`_generated.` basename infix, or a do-not-edit-style
      header (`@generated`, `do not edit`, `autogenerated`) in the first 10
      lines. Otherwise a 3k-line tauri-specta bindings file reads as one giant
      "function" and dominates every fn_length/complexity tail metric. The
      `files(glob)` reader does NOT filter — project gauges choose their own
      corpus.
    - **Language-idiom metrics** (`oxplow.<lang>.*`) — concepts specific to one
      language: **Rust** (`unsafe_blocks`, `unwrap_expect_calls`,
      `panic_macros`), **TypeScript** (`any_usage`, `non_null_assertions`,
      `console_calls`, `ts_ignore`), **Clojure** (`defn_count`), **C#**
      (`empty_catch`, `blocking_async_calls`).

    This repo dogfoods the language-idiom Rust/TS sets + all four unified code
    metrics in its own `.oxplow/project.yaml`. The
    complexity/`code_metrics()`-backed gauges and the C# grammar
    (`tree-sitter-c-sharp` → `Language::CSharp` in `oxplow-code-metrics`) landed in
    tsk229/tsk230.
  - **user-global** — `global_config_dir()/{metrics,gauges,measures,dimensions}/*.yaml`,
    shared across projects, hot-reloaded by the config watcher. Global gauges +
    measures are active everywhere automatically; a global *metric* is enabled
    per-project with a `use:`.
  - **project** — `.oxplow/project.yaml` + gauge scripts under `oxplow/gauges/`.

  `use:` references a catalog metric key and layers threshold overrides; `key:`
  defines a new spec. `oxplow.*` is reserved for built-ins (a project may `use:`
  one but not `key:`-define under it). Gauges are definition-only (declared, never
  `use:`d).
- Validation mirrors the plugin rules: namespaced keys, project-relative
  `entryFile` (no `..`), known runtime/aggregation/displayKind/trigger/direction;
  a `key:` metric must set exactly one of `sourceMeasure`/`formula`; a `use:` with
  an unknown key resolves to a warning (skipped), not an error.

The in-oxplow agent authors these on request via the **`oxplow-metrics`** skill
+ the **`/oxplow:new-metric`** command (assets in `crates/oxplow-plugin/`,
materialized for Claude/Codex/opencode) — "make a metric that counts TODOs" →
the measure+gauge+metric trio + script + verification, no oxplow-team involvement.
`MetricsService::scaffold_metric` writes that trio (measure `<key>.count`, gauge
`<key>`, metric `<key>`) + a starter fact-emitting gauge script.

## Targets & feedback (advise-only, P5/tsk220)

A definition's `target` / `warn_at` / `fail_at` (interpreted via `direction`) are
the single source of red/green: the Metrics page colors from them
(`MetricsPage.statusColor`, three tiers) — no hardcoded UI ramps (the coverage
50/80 ramp is retired; the thresholds live on `oxplow.coverage.diff_pct` as
`target: 80` / `fail_at: 50`, set in `collection.rs::record_coverage_metric`).
See [theming.md](./theming.md).

Feedback is **advisory — oxplow never blocks**. Two paths:

- **Coverage-target nudge** (PostToolUse). When an effort's diff coverage lands
  below target, the ride-along fires a **one-shot** nudge ("coverage X% < 80%
  target — add tests…") via the same `persist_nudge` + `additionalContext` path
  as the report-less / commit-hygiene nudges, deduped per-effort by an in-memory
  `nudged_coverage` set.
- **Effort metric deltas** (UserPromptSubmit, tsk231). `CollectionService::
  effort_metric_context(thread)` builds a "# Metric deltas (this effort)" block —
  for every **code** metric (operational `agent.*`/`effort.*`/`task.*` and `event`
  kinds are skipped) it shows `title: baseline → current (Δ ±N)`. It consumes the
  **shared `effort_metric_deltas` core** (tsk253), so the prompt and the task-page
  panel report the **same** numbers — file-attributed for gauges, so under
  overlapping efforts the agent sees only its own effort's effect, not the repo
  total. The first turn a **gauge crosses** its `warn_at`/`fail_at` (`threshold_state`,
  interpreted via `direction`) the line gets a loud `⚠ crossed fail/warn
  threshold` marker, **one-shot** per `(effort, metric)` via an in-memory
  `nudged_gauge` set — on-snapshot gauges run outside any hook, so the crossing
  can't ride the PostToolUse return; the per-turn prompt context surfaces it
  instead. The control-plane joins this with the session-context block into the
  one UserPromptSubmit `additionalContext`. Returns `None` (adds nothing) when no
  metric moved and no crossing is fresh, so steady-state turns stay quiet.

## Gotchas

- **Provenance is the spine** (carried from collection.md): in-process/parsed →
  `observed`; agent-asserted / exec-tier → `asserted` / `plugin-exec:<name>`. The
  UI must never let an asserted number pass for a measured one.
- **Best-effort writes**: producers swallow metric errors so they never break
  the host (collection ride-along, Stop hook). A missing sample is a logged warn,
  not a failure — check daemon logs if a metric doesn't appear.
- **Raw integer ids at the store layer**; the service/IPC boundary maps to/from
  prefixed domain ids (`str1`, `eff1`). `stream_id` is the hard CASCADE scope;
  `thread_id`/`effort_id` are nullable durable/overlay dimensions.
- **TS bindings + event variants regenerate** via the `export_ts_bindings` test
  in `oxplow-tauri-ipc` (`cargo test -p oxplow-tauri-ipc export_ts_bindings`); CI
  fails on an uncommitted diff.
