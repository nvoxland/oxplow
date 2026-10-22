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

> **Direction (epic tsk275):** this fact substrate becomes the "facts"
> half of the [semantic layer](./semantic-layer.md), which adds entities,
> expression/join dimensions and a read-only `v_*` SQL contract. The
> Metrics pages move into the `oxplow-bundled` extension
> ([extensions.md](./extensions.md)); the substrate and engine stay core.

## The fact substrate (epic tsk12 — the inversion, in flight)

**The defect being fixed.** V38 has facts and metrics *inverted*. `metric_sample`
is labelled "the BI fact grain" but holds *pre-aggregated, per-metric* values
(a count-over-threshold, a `tree:.` repo sum) — aggregation baked in by the
collector. `metric_finding` holds the *atomic, re-aggregatable* facts
(function→complexity, test-case→pass/fail, lint hit) — but ephemerally, as
CASCADE-with-run drill-in, not as a durable queryable series. So a new metric
over the same reality needs a *new collector that re-walks the code*: aggregation is
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
>   snapshot**. This is what a **tree-scanning fact collector** does: after the initial full index,
>   every snapshot is a per-commit **delta** (5–19 files).
> - `capture_scope = per-subject` (V55, tsk43) — a capture restates **only the
>   subjects it emitted facts for**. `oxplow.test_case`: a **partial** run
>   (`bun test src/foo.test.ts`) holds just those cases, so read as `complete` the
>   metric would report *"the repo has 4 tests"* and lose every failure elsewhere.
>   Folded to the **latest fact per `(producer, subject)`**, a partial run updates
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
>   `oxplow.metric.record`, synthetic writes). Its snapshot is **provenance only**,
>   never a scanned set; the insert coerces any snapshot-less capture to
>   `asserted` (delta/full require an anchor).
>
> The scanned set never comes **from the facts a scan emitted** — which is why
> the whole thing needs *no* write-side convention:
> - a file whose count drops to **0** emits no fact (`if c > 0:`), but its path is
>   in the new snapshot ⇒ the new capture supersedes the stale value. **No
>   zero-emission convention; the bundled collector scripts are untouched.**
> - a **deleted** file's latest row is a `storage='deleted'` tombstone ⇒ dropped.
>   **No tombstone facts.**
> - **symbol**-grained facts and **many-facts-per-path** (TODO markers) are
>   superseded *wholesale per file*, so a removed function/marker disappears.
> - partitioning by **producer** matters: the 10 idiom collectors share
>   `oxplow.ast_hit` (sliced by `rule`), so without it a later collector's capture would
>   supersede an earlier collector's facts for the same path.
> - partitioning by **stream** matters for the same reason (tsk98): a stream is a
>   **worktree**, and the fold reconstructs *one worktree's tree*. Two worktrees
>   running the same collector share `(producer, path)` keys, so a stream-blind state
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
> **Baseline (tsk71 — no fabricated snapshot).** A snapshot collector's repo-wide total needs it
> to have restated the whole tree once — that's a **`scan_kind='full'` capture over
> the reconstructed tree of an ordinary snapshot** (corpus via
> `SqliteSnapshotStore::list_tree_files_at` → `build_full_file_map`), NOT a
> fabricated full-tree snapshot. The old `enqueue_full_tree` (mark every path dirty,
> capture a snapshot listing the whole tree) is **gone**: that snapshot polluted
> effort file-attribution — an edit from effort A that hadn't been snapshotted yet
> first landed in a snapshot inside whatever effort window was open when a rebuild
> ran, misattributing files.
>
> `collectors_needing_baseline` is the **pending-baseline queue**: the on-snapshot sweep
> (`run_snapshot_collectors`) partitions fact collectors every time a snapshot lands — already-
> baselined collectors run `delta` over the snapshot's own rows; queued ones run `full`
> over the reconstruction. So a newly added/edited collector baselines on the next
> ordinary snapshot automatically. **`Services::rebuild_metric_baseline(force)`** is
> the on-demand entry point — it waits for the startup sweep, drains genuinely
> pending edits into a NORMAL snapshot (real authored work, correctly attributed;
> none dirty ⇒ **no snapshot is created**, it anchors to the latest existing one),
> then runs the sweep. **Boot, the `oxplow.metric.rebuild` command, and the end-to-end
> tests all call it** (tsk50) — see
> `rebuild_does_not_fabricate_a_snapshot_on_a_clean_tree`. **Not** needed on a
> branch switch — checkout rewrites the differing files, the watcher marks them
> dirty, and the delta rescans exactly those paths.
>
> The sweep is **idempotent per (snapshot, collector, fingerprint, scan_kind)**
> (`collector_done_for_snapshot` — kind-scoped so a delta capture can't satisfy a
> pending full baseline over the same snapshot): a repeat rebuild or a redelivered
> `snapshot.taken` won't double-scan the tree (an explicit `oxplow.collector.sync`
> bypasses it — "run now" always runs).
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
> baseline survive, asserted/failed captures survive, and so does a producer
> with no per-path fact anywhere — a whole-tree restate of a complete-scope
> measure (`oxplow.duplicate_lines`, empty on a clean tree) is history, not a
> baseline (tsk709; before that rule each commit pruned the previous empty
> restate and wiped the stream's cube). The post-sweep prune runs only when
> the sweep baselined a `needing` collector. Accepted trade-off: a
> per-path measure's TREND loses pre-baseline points; the current fold and every
> effort window at/after the baseline are unaffected. Read paths are also
> bounded SQL-side now (`facts_for_measure_in_stream`, `pathless_scalar_facts`,
> `representative_facts_by_slice`, pinned findings via `facts_for_captures`) —
> never "load the whole measure history and filter in Rust" on a hot path — and
> `v_effort_metric_delta` refreshes are debounced (the OTLP token tick fires
> `MetricSamplesChanged` every ~10s while an agent runs).
>
> **Every `metricSamplesChanged` listener needs a bound on how often it
> reloads — it bit twice (tsk91); today that is the measure scope plus
> single-flight below.** `RecordedMetricsPage` + `MetricsExplorerPage` reloaded un-debounced,
> so oxplow burned **~20 CPU-seconds per agent tool call** (bursting to ~500% /
> ~200 threads, profiled straight to `row_to_fact_row`): a reload is one
> `listMetricSamples` per catalogued metric, fired as ~40 concurrent blocking
> reads, and **each walks its measure's whole history** — `oxplow.test_case` is
> ~235k facts (+~5k per `test:collect`) and yields ~118 points, one per capture.
>
> **The full mitigation stack (tsk191 idle-CPU profile → tsk196/197/198).** The
> ~10s OTLP token tick was still spiking to 400%+ while "idle". Three composing
> fixes, cheapest read to root cause:
> - **Reads-based refresh** (P4.7, replacing tsk197's `subscribeMetricRefresh`).
>   Each metric reader in `api.ts` returns its rows with the query's `reads`,
>   and every metric view (`MetricTile`, `MetricDetailPage`, `MetricsPage`,
>   the dashboard's spec and catalog load) re-runs through
>   `useRerunOnChange`, so a view refreshes only when a model or measure it
>   read changed. An enable toggle or a `project.yaml` edit arrives as
>   `modelsChanged` for `v_metric_catalog` / `v_metric_spec`. `MetricsPage`
>   reads every metric, so its refresh sits behind `coalescedRefresh`
>   (single-flight): a refresh never overlaps the one before it.
> - **Cube read cache** (`cube_rows_for_measure`, tsk196). The read is memoized on
>   a new **`metric_cube_epoch.version`** counter — distinct from the `epoch`
>   fence below, which deliberately does NOT move on a fold. `version` is bumped
>   by AFTER INSERT/UPDATE/DELETE **triggers** on `metric_cube` (so FK cascades
>   count too), giving exact, staleness-free invalidation. Collapses N-tiles →
>   1 read per event.
> - **Measure-scoped event** (tsk198). `MetricSamplesChanged` now carries
>   `measures: Vec<String>` (the keys the write touched). A consumer that declares
>   its own `source_measure` skips an event whose measures it doesn't read — so a
>   token export stops waking a coverage tile. **Fail-open both ways:** an empty
>   event list or an unscoped/formula consumer refreshes on anything. **One
>   emitter** (P7.B1): the change loop (`models_changed.rs`, `CaptureListener`)
>   announces it whenever a commit touches `metric_capture` or `fact`, per stream,
>   naming the measures of the facts that landed since it last read; no recording
>   site emits it, so none can forget or misname it (a source-tree test holds
>   that). A capture with no facts is announced with no measures — correctly
>   fail-open, since a capture with no facts can still move a series (supersede /
>   zero-fill, tsk41/tsk44) in ways the facts can't reveal.
>
> **Before optimizing anything in this path, read
> [performance.md](./performance.md)** — it has the profiling harness (which
> doubles as the correctness gate), two samply traps that silently produce a
> wrong ranking, what is already optimized, and what measurement has ruled out.
>
> **Bounded reads — Phase 1 (tsk202/tsk204).** The read shape is now windowable.
> the series reads take a time window (today `metric_grid` over a range), and
> a `TimeWindow` threads through `series[_for_spec]_in_stream` → `cube_series`.
> **The cube windows safely for EVERY scope** — each cube row already carries its
> capture's fully-folded state, so dropping out-of-window captures never changes
> an in-window point (a per-subject 11:00 point still folds in a subject last
> restated at an out-of-window 10:00). Complete-scope fact fallbacks window too
> (they have no empty captures, so zero-fill can't diverge). The frontend fetches
> the **widest preset window** (`widestPresetWindow`, ≤30d) instead of the whole
> history, then switches among narrower presets client-side (`filterByRange`) with
> no re-fetch. `apply_window` is the exact old client filter, moved server-side.
>
> **Phase 2 (tsk205) — memoizing the FACT-FALLBACK fold.** `oxplow.high_complexity_fns`
> / `oxplow.long_functions` (and any per-path/per-subject measure sliced off the
> cube) **bypass the cube** — their `min_value` threshold filters each fact's own
> value, which the cube summed away — and replay the whole history via
> `tree_state_series`. A window can't bound that read (the newest point still needs
> the from-start fold to seed pre-window state), so instead the UNWINDOWED fold is
> memoized per `(measure, read shape)` for one generation, and the window is a
> post-filter. N metric views refreshing off one event now cost ONE fold, not N.
>
> ⚠️ **The freshness token is CAPTURE-scoped, not fact-scoped — this is the whole
> subtlety.** The first cut keyed on `MAX(fact.capture_id)` for the measure and was
> wrong: a rescan that finds a file clean emits **no fact** and supersedes the old
> count with 0 (tsk41/tsk44), so the fact max never moves while the series changes.
> `rescanning_a_fixed_file_supersedes_its_facts_and_drops_the_metric_to_zero` caught
> it. `capture_token_for_producers` counts the measure's producers' captures
> (`COUNT` + `MAX`, so a delete registers too), which sees empty captures — and
> stays producer-scoped so the ~10s `otel-tokens` captures don't evict a complexity
> series they cannot affect. An empty producer set (the `oxplow.lint_hit`
> clean-analyzer seed, whose capture axis comes from elsewhere) is **not memoized at
> all** rather than risk a stale read.
>
> Still open: this makes repeated reads free but the FIRST read per generation still
> folds. Eliminating that needs per-capture points materialized during the cube
> build (which already holds the live partition) — re-measured in P12
> (performance.md, "Bounded series reads, re-measured"): three threshold counts,
> ~0.5–0.7 s each, are what's left (tsk1019).
>
> **Fact collectors must be able to FINISH a whole-tree scan, and a failure must be seen.**
> The script `Budget` (`oxplow_domain::run`) default (5s) is sized for a report parser over one file. A tree
> collector tree-sitter-parses the *whole tree* per run, so fact-collector runs get their own
> ceiling (`FACT_COLLECTOR_TIMEOUT`, 120s). Under the old 5s budget the broad-query collectors
> timed out on every full-tree run and wrote **nothing** — `oxplow.ts.console_calls`
> and `oxplow.ts.ts_ignore` had produced **zero facts since the project was indexed**,
> against a repo with 137 console calls, and the only trace was a `tracing::warn`
> (tsk47). A failing collector now records a **`status='failed'` capture** (with the
> error and the fingerprint), and a whole-tree sweep is a tracked
> `BackgroundTaskKind::Metrics` task with per-collector progress that **fails** if any
> collector failed (tsk48) — so "why is oxplow pegging a core" and "is this metric
> trustworthy" both have answers.
>
> ⚠️ **Non-`done` captures are invisible to every fold** (`c.status = 'done'` in
> `latest_tree_facts` / `latest_subject_facts` / `scanned_paths_for_captures`, AND in
> `captures_for_producers` — the in-memory fold's and the cube build's capture
> source; the tsk103 review found the latter unfiltered, so the claim used to be
> false for those two). This is load-bearing: a failure capture carries **no
> facts**, and on a full-tree snapshot it restates *every path* — so if the fold
> counted it, one timeout would supersede everything and silently zero the
> metric. Worse than the bug it reports.
>
> `needs_tree_baseline` asks ONE question **per collector** (`collectors_needing_baseline`):
> *does this collector have a completed `scan_kind='full'` capture at its current logic
> fingerprint?* (`SqliteFactStore::has_full_capture`; fingerprint =
> `collector_fingerprint` — a hash of script text + runtime/report + `facts` (+ the
> `input:` query when set), stamped on every capture as
> `metric_capture.producer_version`, V56; its `v1` material is exactly what the
> pre-P7.B3 gauges hashed, so a migrated gauge keeps its baseline). That single check
> covers BOTH a fresh/never-baselined collector (incl. one stuck on deltas because its
> full scan used to time out, tsk47/tsk49) AND a script change since the last
> baseline (a full capture at stale logic carries the old fingerprint, tsk45). An
> unfingerprintable script matches any-version — one full capture ever.
>
> **Per-COLLECTOR, not per-measure, is load-bearing (tsk49).** `oxplow.ast_hit` is one
> measure shared by 10 idiom collectors (sliced by `rule`), so "does the measure have
> facts" says nothing about one collector — a delta-only collector looks done because a
> *sibling* filled the measure. That is exactly how `oxplow.ts.console_calls` read
> empty for weeks (137 real calls): `unsafe_blocks` completed its full-tree scan, so
> `ast_hit` wasn't empty, so the old measure-level check never re-baselined the heavier
> TS collectors that had only ever run on deltas.
>
> **(2) is not optional either.** A collector's facts are only as good as the code that
> computed them, so a script change makes them stale but *not* empty. Without the
> fingerprint a metric fix **silently no-ops** — you correct the query, the number
> doesn't move, nothing says why (tsk44→tsk45: teaching `repo_allow.star` to match
> inner `#![allow(...)]` changed nothing until the captures were hand-deleted).
> Re-baselining restates every path, so the fold supersedes the stale facts — no
> deletes, history preserved.
>
> `per-path` today: `oxplow.ast_hit`, `oxplow.complexity`, `oxplow.fn_length`,
> `oxplow.parameter_count`, `oxplow.todo` (+ any project measure a snapshot collector
> records per-file facts on — the scaffolded metric extension sets it). Validated in
> config + `CaptureScope::parse`, deliberately **NOT** a DB CHECK: the
> `temporal_semantics` CHECK is exactly why adding a value *there* would need a
> `measure` table rebuild, which fires `fact.measure_id ON DELETE CASCADE` and wipes
> every fact (see V52).

- **`measure`** — the namespaced catalog of *fact types*: `key` (`oxplow.*`
  reserved), `title`, `unit`, `subject_kind` (the grain; a resource name, `oxplow/effort`, for the facts core records), `capture_scope`
  (`complete` | `per-path`, V54 — see the box above), `temporal_semantics`
  (`additive` | `semi-additive` | `non-additive` — additivity **over time**:
  tokens additive; complexity + test/lint SNAPSHOTS semi-additive (a run
  replaces the last — V47/tsk42 fixed test_case/lint_hit from V43's wrong
  `additive`); a **level ratio** whose every capture restates the value
  (coverage) is *also* semi-additive — the headline is the latest capture's
  Σn/Σd, not a history blend (V50/tsk13 fixed coverage from V43's wrong
  `non-additive`); only the **accumulating** mean-across-closes ratios
  (cycle_time, effort — one observation per close, Σ over all captures =
  the mean) are `non-additive`),
  `scope`, `description`. (Ratio components ride each fact's num/den; the
  dead V43 `component_role` column is dropped in place by V159, tsk865 —
  `ALTER TABLE … DROP COLUMN` rewrites the table without a rebuild, so the
  `fact→measure` CASCADE never fires.)
  oxplow's own measures (below) are declared in **oxplow-foundation**'s
  `measures:` (scope `extension:oxplow-foundation`; the migration kept the
  seeded rows and restated their scope): `oxplow.complexity`, `oxplow.fn_length`,
  `oxplow.parameter_count`, `oxplow.todo`, `oxplow.coverage`, `oxplow.test_case`,
  `oxplow.lint_hit`, `oxplow.duplicate_lines`, `oxplow.tokens`,
  `oxplow.cycle_time` (V43), plus `oxplow.coverage.branch` +
  `oxplow.coverage.function` (V68, tsk123 — per-file branch/function coverage
  ratios beside line coverage), plus `oxplow.ast_hit` (V45 — a per-file AST idiom
  occurrence; the per-language idiom collectors record facts on it, distinguished by the
  `oxplow.rule` dim; see "Per-language idiom collectors", tsk30), plus
  `oxplow.effort_test_outcome` (V53, tsk38 — a per-effort-close scalar the
  foundation's `oxplow.effort_test_outcome` collector records when the effort
  finishes; the four `oxplow.tests.{failed_at_close,
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
  `captured_at`/`ended_at`; where `snapshot_id`/`closest_vcs_rev`/
  `vcs_rev_exact`/`branch`/`basis_ref`; who `stream_id` (NOT NULL, the CASCADE
  scope) / `thread_id` / **`effort_id`** (nullable, `ON DELETE SET NULL` — the
  *producing* effort: the tool call's effort, else the thread's open one);
  trust `provenance`/`source`. **Captures are durable by default** (they carry
  the facts' context — no independent sweep). The one opt-in exception is
  `metricRetentionDays` (tsk93, **default 0 = keep everything**): when set, a
  daily pass (`prune_aged_captures`) deletes captures older than the window
  that no current value stands on — effort-stamped captures, each producer's
  newest, and any capture owning a fold-live fact are always kept, the
  affected cube is invalidated + epoch-fenced in the same transaction, and
  the trade is explicit: series/drill-down/flakiness history beyond the
  window is gone. Turning the knob is watching-and-deciding territory, not a
  default.
  >
  > **Detail COMPACTION is the on-by-default sibling (tsk211).** Distinct from
  > the prune above: `compact_capture_details` only NULLs `metric_capture.
  > detail_json` — the verbatim per-run payload behind the effort Tests/Coverage
  > panel. The capture row and every fact survive, so no metric
  > value, trend point or count can change; that is exactly why it defaults ON
  > while the fact-deleting prune stays opt-in. Dedup is unaffected —
  > `idempotency_key` is derived from `detail_json` at WRITE time into its own
  > column, so a replayed report still matches after the payload is gone.
  >
  > Two bounds, either of which compacts a capture:
  > `metricDetailMaxPerProducer` (default 100) keeps detail for the newest N
  > captures PER PRODUCER, and `metricDetailRetentionDays` (default 30) drops it
  > by age. The count cap is the one that bounds a busy repo — measured here,
  > `detail_json` was **200 MB of a 795 MB database** (`tests` 103 MB, `coverage`
  > 97 MB at ~0.5 MB/run) accumulated in under three weeks, while **nothing was
  > older than 30 days**. Age alone would have reclaimed zero. Both run in the
  > same daily `boot.rs` loop as the prune, but independently of it — the loop
  > used to be gated on `metricRetentionDays > 0`, which would have disabled
  > compaction for everyone.
  > **Stamp `closest_vcs_rev` on every capture you add (tsk95).** Use
  > `file_ref_version::resolve(store, dir, snap)`: a snapshot with its own commit
  > reads `vcs_rev_exact = true`, otherwise it falls back to HEAD with
  > `exact = false`. **Dirty is the normal case** (the agent edits, then runs
  > tests), which is exactly what the `closest_vcs_rev` +
  > `vcs_rev_exact` pair is for — don't add a third field.
  >
  > **But commit ancestry alone cannot place a dirty run (tsk97, verified).** A
  > dirty run on a feature branch stamps the *fork-point* commit — which is on
  > main — so "visible iff ancestor-or-equal" keeps the branch's results visible
  > from main: the exact cross-contamination it was meant to prevent. Hence the
  > fold partitions by **branch** (see the cube box below), and a future
  > ancestry resolver must anchor a dirty run to the commit that **absorbed** it
  > (the next snapshot carrying a commit), never the one it branched from.
  > That IS recoverable: every test capture carries a `snapshot_id`, snapshots
  > carry `revision`/`branch`, and the git-refs listener re-stamps a
  > stream's latest snapshot on commit — the "permanently ancestry-blind
  > captures" this box used to claim were verified to be zero.
- **`fact`** — the durable atomic measurement (folds `metric_sample` +
  `metric_finding`): `capture_id` **NOT NULL** (→ all context via the capture),
  `measure_id`, `value`, `numerator`/`denominator`; `subject` (one URI: `oxplow:oxplow/file?path=src/a.rs`, `oxplow:oxplow/symbol?path=src/a.rs&symbol=f`,
  `oxplow:oxplow/ai_model?name=claude-opus-4-8`; a write refuses a string that isn't a URI), `path`/`line`
  (location-at-capture); reported finding metadata
  `severity`/`rule`/`detail` (null for pure measurements); `dims_json` (long-tail
  dims). **No when/where/who columns** — those are the capture's.
  **Stored as ids into dictionaries (V38):** `subject_id` → `fact_subject
  (ref, kind)` (the whole ref, unique; `kind` derived from it by the store at insert,
  a filter convenience and never an identity — V64), `path_id` → `fact_path`, `dims_id` → `fact_dims (json)`;
  the one writer (`insert_fact`) interns, and `FactRow` still carries the
  text — except `detail`, which bulk reads leave out (`None`; a finding's
  message is fetched by id with `SqliteFactStore::fact_details`). A fact repeated its subject, path and dims text on every row —
  ~290 bytes a fact with indexes, 3.1 GB of a 4 GB database — while the
  distinct values were ~58k subjects, 1.7k paths and 730 dims sets; the
  fact data went 1.83 GB → 0.52 GB. Reads select the ids and
  `fact_row_mapper` resolves each once per read (`FactText`) — per-row
  SQL lookups cost a quarter more read time. A query that filters,
  groups or partitions uses the ids where they're equivalent (`path_id`,
  `dims_id`) and joins `fact_subject` where the subject ref's text is the
  key. `v_fact` and `v_tree_fact` join the
  dictionaries, so their columns are unchanged.
  **Complete-scope scans store only what changed (V39, `fact_chain.rs`).**
  A finished capture of a complete-scope measure stores the facts that are
  new or changed since its producer's previous finished capture in the
  same stream (its base), and **holds** the rest: a fact is held by its
  chain's captures from its own capture up to `last_capture_id` (NULL
  while the newest holds it), and a `fact_chain (capture, measure,
  from_capture_id, depth)` row marks a holding capture. A dropped fact is
  closed at the base. A capture stores the measure whole (closing what the
  base held, `from_capture_id` = itself) when there's no chain to extend,
  when more than half of what it restates changed, or every
  `MAX_CHAIN_DEPTH` (32) captures, so a read looks back a bounded range.
  Every fact read goes through `held_facts_sql` (own rows of captures
  with no chain row, plus each chain row's held facts, with the
  **holder's** capture columns), and `v_fact` spells out the same rule,
  so readers still see each capture's whole set. **Consequences:** a fact
  id repeats across the captures holding it unchanged (`v_fact` v2); a
  query that joins `fact` to `metric_capture` on `f.capture_id` sees only
  what a chain capture stored, so a new reader of complete-scope facts
  goes through `held_facts_sql` or `v_fact`; the chain join is pinned with
  CROSS JOINs, spelled-out bounds and named indexes (see performance.md).
  Failed and asserted captures and per-subject measures store as before,
  held by their own capture only. Pruning moves a fact a doomed capture
  stored up to the first kept capture that holds it (`rehome`, only a
  holder whose `from_capture_id` is at or before the fact), and prune's
  keep rules protect a fact's **last holder** (`LAST_HOLDER`), not the
  capture that stored it. `prune_dominated_tree_captures` never takes a
  chain holder, and rehomes before it deletes. The live database's history
  was rewritten into chains once, by hand (the same diff, replayed in
  capture order), when V39 landed.
  **Per-path code gauges hold each rescanned file the same way (V43).** A
  finished delta or full scan of a per-path measure writes, per file it
  emitted facts for, a `fact_path_hold (capture, measure, path,
  from_capture_id, depth)` row: it holds that file's facts stored since
  `from_capture_id` and not closed before it, and its own rows for the
  file are only what's new. The diff against the file's last hold matches
  on everything **but the line**, because an edit shifts the functions
  below it (on this project a rescan's facts were 60% identical, 99%
  identical but for the line): a fact that only moved stays one fact, and
  `fact_line (fact, capture, line)` says where it is from that capture on —
  a holder reads the newest one at or before it, else the fact's own line,
  so every capture's drill-down keeps exact lines. A dropped fact is closed
  just before the scan (nothing between the last hold and it holds the
  file). A file restated with no facts gets no hold, so the scan holds
  nothing for it — what the tree fold expects. The tree fold
  (`latest_tree_facts`, `v_tree_fact`, so `v_file_metric` and
  `v_function`) reads what each path's winning capture **holds** for it;
  `held_facts_sql` and `v_fact` read path holds as a third branch. A
  rehomed fact takes the line it has at its new home and its earlier
  `fact_line` rows go. The live history was rewritten into holds once, by
  hand, when V43 landed.
- **`metric_cube`** + **`metric_live_fact`** + **`metric_cube_state`**
  (`V62__metric_cube.sql`, tsk96; live state + watermark re-keyed per **branch**
  by `V63__branch_aware_cube.sql`, tsk97) — the **aggregate cube**: the
  materialized fold. See the box below.

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
> - **a value threshold no spec of the measure uses** — `min_value` /
>   `max_value` (`oxplow.high_complexity_fns` 11, `oxplow.long_functions` 61,
>   `oxplow.coverage.untested_files` 0) filter on a fact's OWN value, which
>   the cube summed away. The bounds the measure's specs use ARE in the grain
>   (below), so those specs are served; any other bound is read from facts.
> **Value bounds are part of the grain (tsk1019).** A bucket is the facts that
> agree on the promoted dims *and on whether they pass each bound a spec of the
> measure uses*: `dims_key` carries `"value>=11":"1"` / `"0"` per bound
> (`ValueBound`, `metric_cube.rs`), computed with the test `FactFilter::matches`
> makes. A threshold count is then a sum over the buckets that pass — bucketing
> is bounded by the bounds specs declare, never by distinct values (the "fact
> table again" objection). The fold stays filter-agnostic, so a subject
> crossing the bound in either direction is restated exactly as before. The
> builder derives the bounds from the measure's `metric_spec` filters on every
> pass; a cube whose rows carry a different bound set is dropped and folded
> again (`reset_cube`), and the read declines a bound its rows lack. Reads of
> the three declined specs: ~230 ms from facts → ~1 ms
> ([performance.md](./performance.md) "Threshold counts from the cube").
>
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
> **The live state is partitioned per `(stream, BRANCH)`** (V63, tsk97),
> mirroring the fact fold (50fd1760): a capture evicts/inserts only within its
> own branch's partition — else a feature branch's failure lands on a point
> labelled `main` whenever main didn't re-run that test — and a branch's FIRST
> capture **seeds** its partition by replaying the history visible to it
> (`seed_rows`, one in-memory replay per new branch, written in one
> transaction), so a new branch **inherits** the pre-fork suite instead of
> collapsing to what it re-ran. `metric_cube` itself needs **no** branch column:
> its grain is the capture, and the capture carries the branch.
> `the_cube_keeps_each_branchs_state_separate_and_a_new_branch_inherits` pins
> both halves — it began life as the decline guard that refused multi-branch
> reads while the build lagged the fold, and was FLIPPED, not deleted.
>
> **"Visible to it" is the ancestry rule** (tsk102, `metric_visibility.rs` —
> read `Visibility`'s type docs in `metric_engine.rs` for the rule itself):
> same branch always sees; cross-branch, C is visible from R iff C's
> **absorbing commit** (`effective_commit` — the first same-stream, same-branch
> commit-stamped snapshot at-or-after C; an exact capture is its own anchor) is
> an ancestor-or-equal of R's **base** (`closest_vcs_rev` — tsk95's stamp
> IS the base, which is why capture-level stamping stays). Three load-bearing
> properties, each pinned by test:
> - **Never anchor a dirty run to its fork point** — `closest_vcs_rev` is
>   an ancestor of everything, so ancestry over it cannot separate branches
>   (tsk97's disproof). The anchor is the commit that ABSORBED the work.
> - **As-of-R with the absorbing COMMIT's own timestamp**: work not yet
>   absorbed when R ran reads visible, permanently — which makes every
>   resolved `(C, R)` answer IMMUTABLE (new commits only affect future
>   readers), so the cube's frozen seeds can never diverge from a fresh
>   fact-path read and no invalidate-on-commit machinery exists or is needed.
> - **Unresolvable ⇒ visible** (missing branch/stamp/sha, unreadable repo) —
>   never invent strictness from missing data; degraded = pre-tsk102 blind.
>
> **One `VisibilityResolver` instance feeds every fold** — the engine's fact
> fold and the cube's seed share `AppState.metric_visibility`, and
> `CollectionService` (effort deltas) reads through the app's one
> `MetricEngine`, never a per-call engine — so the fold memo and the
> ancestry cache are shared too. One side resolved with the other blind is how the cube
> silently diverges — `the_cube_seed_and_the_fact_fold_resolve_ancestry_
> identically` pins the pair. Known, symmetric limitation: cross-branch
> results flow only through the seed at a branch's FIRST capture, so work
> merged INTO an already-seeded branch never retro-appears on that branch's
> points — in either path, by the same immutability that protects the cube.
> It self-heals as the branch re-runs those tests, and a branch forked after
> the merge seeds with the merged history.
>
> **The watermark** (`metric_cube_state`) exists because "no cube rows for capture
> N" is otherwise ambiguous: state legitimately empty at N (a real value-0 point)
> vs N not cubed yet (fall back). Conflating those is how a materialized read
> reports 0 instead of admitting it doesn't know. It also makes the build
> **crash-safe**: the cube is written outside the fact-insert transaction, and
> since tsk113 the build folds captures in CHUNKS — the in-memory fold's
> evict/insert per capture, one `apply_build_batch` transaction per ~256
> captures (the profile showed ~10k per-capture transactions rewriting the
> same hot WAL pages; batching halved the backfill). A torn chunk lands
> NOTHING and leaves the watermark un-advanced — reads fall back and the next
> build replays whole captures, which is idempotent (evict+insert *replaces* a
> subject's facts).
>
> Watermark rows are per `(measure, stream, branch)` (V63) and do double duty:
> the STREAM's watermark — what the read checks coverage against — is the MAX
> across its branch rows (valid because the build processes a stream's captures
> in global `(captured_at, id)` order), and a row's **existence** is the
> branch-seeded marker, keeping "seeded but legitimately empty" distinct from
> "never seeded". A crash between seed and first row-write simply re-seeds —
> the seed is a transactional replace.
>
> **Anything that changes what a replay would compute must invalidate the cube**
> — the tsk100 rule, generalized by the tsk103 review from "anything that deletes
> captures or facts". Three invalidators exist, each change-detected and each in
> the SAME transaction as its change:
> - **`prune_dominated_tree_captures`** (tsk100) — facts cascade and
>   `metric_live_fact` self-heals (FK), but `metric_cube` rows are frozen at
>   build time and don't. Only when it actually dropped something —
>   `rebuild_metric_baseline` prunes on every boot, and unconditional
>   invalidation would wipe a healthy cube each start.
> - **`upsert_dimension` on a `promoted` flip** (either direction, and a new
>   dim arriving promoted) — promotion is a GRAIN change; a pre-promotion
>   bucket lacks the key and serves explicit 0s through a newly-eligible
>   filter. The V64 migration honored this by hand; the config path
>   (the generation, every boot) now honors it automatically.
> - **`upsert_measure` on a `capture_scope` change** — the scope picks the
>   build RULE; scoped to that one measure.
>
> Every invalidation also bumps **`metric_cube_epoch.epoch`** (V66): the build runs
> outside these transactions, so a wipe can land mid-pass, and the builder's
> next `write_cube_rows` — carrying the epoch it planned under — refuses and
> abandons rather than re-planting a watermark over captures whose rows the
> wipe deleted ("covered but rowless" would serve explicit 0s). A fenced pass
> costs one re-fold; a fenced write costs nothing. Slow, never wrong.
>
> **Don't confuse `epoch` with `version` (V72, tsk196).** The same singleton row
> also carries a `version` counter, but it is a DIFFERENT tool. `epoch` fences
> WRITERS and moves only on wipes — a fold reads it and must NOT bump it, or
> concurrent folds would abort each other. `version` invalidates the
> `cube_rows_for_measure` read CACHE and must move on EVERY cube mutation, so
> it's maintained by triggers (fold, wipe, and FK cascade alike). Reusing `epoch`
> as the cache key would freeze the metrics UI — folds land, epoch stays put,
> cache serves pre-fold rows forever. Two counters, opposite jobs.
>
> The fold's third input — the `file_snapshot` rows the per-path restated
> sets are derived from — is **durable by construction** since tsk105:
> Local History retention expires only blob CONTENT, never the rows, so
> replay inputs cannot rot out from under durable captures and no
> invalidation is needed for retention. See the data-model doc's Retention
> section.
>
> **The grain's floor is the CAPTURE.** Never aggregate coarser (per-day,
> per-commit): a capture *is* one scan/run, so `snapshot_id`/`effort_id`/
> `thread_id`/`branch`/`closest_vcs_rev`/`stream_id` stay reachable through
> the JOIN and within-effort deltas keep working. Branch/thread/stream remain
> **dimensions you can group by, never partitions that hide rows**.
>
> **`dimension.promoted` = the cube's grain** (tsk28's flag, inert until V62).
> Promoted: `oxplow.status` (V62 — cardinality 2, what `tests.passed`/`.failed`
> filter on) and, since V64 (tsk101), the four filter dims the declining specs
> needed — `oxplow.rule` (13), `oxplow.token_kind` (4), `oxplow.tests_stat` (4),
> `oxplow.severity` (1). Measured grain cost of all five: ~1.4× rows.
> `oxplow.test_suite` stays unpromoted (cardinality 234 ⇒ 18,918 rows, for
> slicing no spec asks for). The dims live on different measures' facts, so the
> grain doesn't cross-multiply — `dims_key` carries only the promoted dims a
> fact actually has. **Gate any promotion on measured cardinality, and treat it
> as a cube rebuild, not a schema change** (V64 clears all three cube tables —
> a pre-promotion bucket merged values the new grain separates, and `metric_
> live_fact` must be rebuilt alongside the watermarks: a replayed capture
> re-aggregates the whole live partition, so leftover final-state rows would
> leak future facts into historical points). The raw facts always keep every
> dim, so nothing is foreclosed by waiting.
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
>   - **partial** (`build_stream`) — folds captures after the watermark in
>     chunks: seed a new branch's partition → load each touched partition
>     ONCE → evict/insert in memory per capture → re-aggregate the **whole
>     live partition** into that capture's rows → flush the chunk as one
>     transaction (tsk113).
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
> - **An asset** (P7.B1, `assets.rs`): the builder is a `Materializer` over
>   `metric_capture` and `fact`, registered at boot. Its first build is the
>   backfill; after that the change loop marks it dirty on every commit to
>   either table and it folds once the burst is quiet (1 s), recording
>   `asset_state`. No bus event, no list of recording sites to keep in step.
>   Failures are logged, never propagated.
>
> **Measured on the real DB (512k facts).** The 5 test specs: **9.26s → 70ms
> (~131×)**. All **68** specs (both scopes, after tsk99): **11.53s → 1.03s**, with
> **zero divergence** from the fact path on any of them. Backfill ~25s once, in the
> background. *That 9.26s every ~10s was the CPU burn.* Re-verified after the
> branch-aware build (tsk97, 590k facts, a genuinely two-branch capture history):
> all 68 identical again, **3.74s → 334ms**, 42/68 served; and after V64's dim
> promotion (tsk101): all 68 identical, **4.07s → 446ms**, 58/68 served. The
> harness is `examples/cube_equivalence.rs` — run it against a **fresh `VACUUM
> INTO` copy** (never the live file, never a copy that's already been built: the
> oracle must be fact-served, or it's the cube confirming itself).
>
> **58 of 68 specs are cube-served** (42 before V64 promoted the four filter
> dims — tsk101). The 10 that decline are all expected classes: 2 `min_value`
> thresholds (permanent, by design) and 8 whose measure or filter matches **no
> facts yet** (clean collectors, zero `severity=warning` rows) — those cube
> automatically the moment matching facts exist, and declining them is correct:
> the fact path owns empty-producer seeding (tsk62). Verify a decline is one of
> these classes before assuming the cube is working.
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
  - A `key:`-defined metric in the project's own extension may set `source_measure` to
    one of oxplow-foundation's measures to add a new aggregation over facts a
    bundled collector already records — no new collector, no collection. E.g.
    `repo.complexity_max` = `max` over `oxplow.complexity`, `repo.fn_length_max` =
    `max` over `oxplow.fn_length` (the key just can't reuse the reserved `oxplow.`
    namespace).
  `direction`/`target`/`warn_at`/`fail_at`/`display_kind` (`gauge`|`findings`|
  `test`|`coverage`|`event` — read-time only; severity/threshold-state are DERIVED
  from `value` × these, never stored on a fact), `scope`/`category`/`language`.
  **Additive** beside the old `metric_definition` (still FK-referenced by the V38
  `metric_sample`/`metric_finding`); the retire migration (tsk20) drops the V38
  cluster once reads flip (tsk26). The migration seeds no rows; the code-metric
  specs come from oxplow-foundation's `metrics:` (`oxplow.high_complexity_fns` /
  `long_functions` / `fn_count` / `todos` — a `count` over its measure,
  thresholds via `min_value` — and the per-language idiom specs such as
  `oxplow.rust.unsafe_blocks`, a `Sum(oxplow.ast_hit)` filtered by
  `dim_eq(oxplow.rule, …)`). Config/global spec seeding lands with the read-flip (tsk26).

`SqliteFactStore` API: `upsert_measure`/`get_measure`/`list_measures`,
`upsert_dimension`/`list_dimensions`, `upsert_spec`/`get_spec`/`list_specs`,
`record_capture`, `record_facts(capture, facts)` (atomic — inserts the capture,
backfills `capture_id` into every fact, commits together; **idempotent** when
the capture carries an `idempotency_key` — a second write with the same key is a
no-op that returns the existing id, so a replayed report never double-counts,
tsk14/V51. `metric_capture.idempotency_key` is nullable with a partial unique
index; the report ingests set it via `CollectionService::ingest_idempotency_key`
= hash(producer + git version + snapshot + verbatim payload). Keyless captures —
fact collectors, tokens, lifecycle — always insert fresh), `get_capture`,
`facts_for_measure` (joined to the capture for the time/version/effort spine),
`facts_for_captures` (the attribution-by-claim read). Ratio re-aggregation is
NOT a store method — it lives in `metric_engine::aggregate_facts`.

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
  effort) would otherwise collapse to a den=0 → 0.0 headline; `compute_rollup(facts, dimension, temporal,
  current_caps)` → `RollupRow`s (deleted in tsk516 — slices are `metric_grid(…, dim)`), additivity-aware like `range_value` (tsk41) and
  scoped to the CURRENT captures (tsk44): semi-additive → only facts in the
  latest capture per (stream, producer) (`current_capture_ids` — else a deleted
  file's stale last fact haunts the breakdown forever) — summed per
  dim value — **unless** the facts carry ratio components (a level
  ratio like coverage, tsk13), in which case the per-group value is Σn/Σd, never
  a sum of per-file percentages; additive → EVERY fact counts (tokens by model
  is a running total, not the last turn); non-additive → current captures,
  per-group Σnumerator/Σdenominator, never a naive
  sum/average of percentages.

  **Currency is `current_capture_ids` alone — there is NO per-subject dedupe**
  (tsk157). That set admits at most one capture per `(stream, producer)`, so
  every fact surviving the filter is from that partition's newest scan, and
  distinct partitions UNION (worktree B never evicts worktree A; one analyzer
  never evicts another for the same path — the tsk98/tsk106 forbidden shape).
  A `latest per (stream, producer, subject)` map on top of that was a no-op for
  tree-scan measures and silently wrong for occurrence-grained ones:
  `oxplow.lint_hit` and `oxplow.todo` emit one fact PER HIT with the containing
  file as the subject, so it made the breakdown count FILES while the headline
  counted hits. The rule is uniform: **any** group whose facts
  have Σden≠0 collapses to Σn/Σd regardless of temporal class.

  **Point-in-time reads describe the headline's own state** (tsk106). For a
  partial-scope measure, `scoped_facts` serves each stream's newest capture's
  `(stream, BRANCH)` partition from the cube's live state
  (`partial_state_facts` → `live_facts`) — visibility-seeded and
  branch-partitioned, i.e. exactly the state the series' last point aggregates,
  so the breakdown/drill-in can never disagree with the headline
  (`a_breakdowns_state_matches_the_branch_aware_headline` pins it). A stream
  the cube hasn't caught up with falls back to the branch-blind SQL folds
  (`latest_subject_facts`/`latest_tree_facts`) — the pre-tsk106 approximation,
  held only while the cube lags. Unscoped reads UNION each stream's own
  current partition (per-worktree states, never merged within a subject);
  note the unscoped *headline* is narrower — the single newest worktree's
  value — a known asymmetry only visible with multiple active worktrees. `dim_value` reads the `severity`/`rule` columns and
  `package`-from-path directly, else `dims_json[key]`. **A dimension has
  one name, its namespaced key** (tsk945, V166): there are no bare aliases
  (`language`, `package`, `branch`, `model`); the fact ingest
  (`record_facts_tx` → `check_dims_tx`) refuses a fact whose `dims_json`
  carries an un-namespaced key, naming the conformed `oxplow.<key>` when
  the catalog declares it. A fact collector asks first
  (`SqliteFactStore::refused_dims`, tsk986): a run whose facts would be
  refused is a **failed run** — its `collector_run` row and a failed
  capture say why, health counts it, a `oxplow.collector.sync` errors — never a
  capture rolled back behind a run reported as recorded. The engine
  refuses a bare key to slice or filter by (`series_in_stream` →
  `bare_dimension`, tsk989), naming the conformed one — a bare key would
  read nothing, an empty slice that looks like no data — and the built-in
  metrics declare only conformed keys. A namespaced key a `dimensions:` entry
  doesn't declare is not refused: it is recorded and slices like a declared
  one, and the entry only names it in `v_dimension`
  (`a_namespaced_dimension_key_need_not_be_declared_to_slice`). Project config
  refuses one in `sliceableDims` or
  `filter.dimEq` (`subject`, the raw-subject pseudo-dimension, aside). V166
  renamed stored bare `language` facts and specs' bare dims, dropped the
  nudge's unread bare `kind` (its subject carries it) and cleared the cube
  to re-fold. `FactRow` carries the
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
  `MetricSpec`, `series_for_spec(spec, group_by)` / `rollup_for_spec(spec, dim)` (deleted, tsk516) /
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

### Spine dimensions and time buckets (tsk321)

- **Spine dimensions.** `oxplow.stream`, `oxplow.thread`, `oxplow.effort`,
  `oxplow.work_item` and `oxplow.vcs_rev` read the fact's capture (arms in
  `dim_value_cached`; excluded in `dim_is_slice_key`, listed in `SPINE_DIMS`).
  `work_item` (the effort's work-item ref) is on `FactRow`:
  `fact_row_mapper(conn)` loads `effort`'s effort→work-item map once per
  read and stamps each row. A per-row join would cost
  a lookup on every fact. They slice and filter (`dim_eq`) on the **fact path
  only**. The cube drops them from the promoted set: a capture's `effort_id`
  is stamped when the effort closes, after the cube may have folded it.
- **Time buckets.** `metric_bucket::bucket_series(points, TimeBucket, temporal,
  agg)` collapses points per (UTC day / Monday week / month, group). A level
  (semi-additive) takes the bucket's last capture; events (additive) sum; the
  rest (non-additive) average; ratios re-divide Σnum/Σden. Each point is
  stamped at the bucket start and carries the bucket's last `capture_id`.
- **One spec read.** `MetricEngine::series_for_spec_read(spec, &SeriesRead
  {group_by, stream, window, dim_eq, bucket})` is the full-option read, and
  `series_for_spec_in_stream` delegates to it. A `dim_eq` on top of a spec's
  own different `dim_eq` is refused (`FactFilter` holds one pair). Buckets are
  applied before the spec's value scale, so ratios re-divide raw parts.

### Entity metrics and entity dimensions (tsk322)

A metric can aggregate a semantic-layer view instead of a measure's facts.

**Config.** A `metrics:` `key:` entry with `entity: v_work_item` (any
view) plus:

- `where` — which rows count;
- `time` — optional; makes it an event metric;
- `value` — needed unless the aggregation is `count`;
- `aggregation` — `count` (default), `count_distinct`, `sum`, `avg`, `min`,
  `max`, `median` or `p90`.

A `dimensions:` entry with `entity`, `expr` and an optional `join` is an
entity dimension. `EntitySpec` / `EntityDimensionSpec` live in
`oxplow-config`. A `use:` can't set the entity fields, and an entity metric
can't also set `sourceMeasure`, `formula` or `filter`. SQL fragments see
the view aliased `e`.

**Storage.** V88 ADD COLUMNs `entity_json` on `metric_spec` and
`dimension`; no table rebuild. The entity's own aggregation lives in
`entity_json`. The stored `aggregation` says how the series' points
combine, which is what the fact path and the detail page's range stat read:

- `last` for a state metric;
- `sum` for an event metric whose buckets add up (count / sum);
- `avg` otherwise, including `count_distinct`: something seen on two days
  counts once, so its buckets must never be totalled (tsk367).

The UI shows the entity aggregation (`specAggregation`).

**Seeding** (`metrics_service::prepare_entity_spec` / `seed_dimension`):

- Each fragment is compiled with `SemanticLayer::check_sql` (prepare
  only, read-only guard). One that doesn't compile is pruned with a
  warning.
- `sliceable_dims` gets every entity dimension over the same view.
- A state metric gets a synthesized measure of its own key
  (`semi-additive`, `complete`); measures are never pruned (their facts stay).
- The built-ins:
  - `work.tasks_completed` (event, `v_work_item.closed_at` of the done
    items, whichever list is active);
  - `work.open_tasks` (state: todo, in progress or blocked);
  - the dimension `work.priority` — the `priority` field a list declares
    (`json_extract(native, '$.priority')`), null for a list without one.

  They are listed in the catalog from code, so a disabled one can be
  re-enabled.

**Reads** (`entity_metrics.rs`):

- **One SQL per read.** Everything aggregates in SQLite and one row per
  (bucket, group) comes back. A read with more than 10k of those (e.g.
  daily buckets × a high-cardinality dimension) is an **error**, never a
  silently short series missing its newest buckets (tsk367).
  - The metric's `where`/`time`/`value` are evaluated in an inner query
    over the view alone. Only a dimension's `expr` sees its `join`, so a
    bare column in `where` can't turn ambiguous.
  - Buckets use `date()`: Monday weeks via `'-6 days', 'weekday 1'`, and
    `start of month`.
  - Median and p90 use `ROW_NUMBER()` windows; p90 is nearest-rank,
    `rn = (9·n+9)/10`.
- **Event metrics** are computed live, daily unless `bucket` says
  otherwise. An ungrouped additive one is zero-filled across the window.
  The headline (`entity_metrics::headline`) of an additive one is one
  unbucketed aggregate over the whole range, so a `count_distinct`
  headline isn't the sum of its daily counts; otherwise it's the latest
  bucket.
- **State metrics** read their captured facts. Their grouped reads are
  live, current value only.
- **Dispatch.** `series_for_spec_read` and `headline_from_series` route
  entity specs to this module. Grouping
  by a non-entity dimension, or `dim_eq`, is refused with an error naming
  the metric's entity dims. Stream scoping doesn't apply (entities are
  project-wide).
- **Effort deltas** skip entity specs (`collection.rs`).

**The `entity-metric` producer** (`MetricsService::capture_entity_states`):

- **When it runs.** Forced at boot and on a reseed (`MetricsService::
  reseed`: a config change through the `config.metrics` reactor, an
  extension change through the catalog's signal). Throttled, at most
  once per 10 minutes per metric, by the `metrics.entity_states` pump
  consumer on `work_item.*`, `snapshot.taken` and `collector.synced`
  (P7.B6; it used to follow the in-memory bus).
- **What it writes.** One fact with the current value on the primary
  stream.
- **When it skips.** When the value equals the last capture: the
  in-memory value, or on a fresh process the latest stored fact. A level
  carries forward, so a restart doesn't pile up duplicates.

### Pacing

An `on:` trigger may pace its runs instead of running on every event:

```yaml
trigger:
  on: [snapshot.taken, effort.finished]
  settle: 30s        # once no triggering event has arrived for 30s
  at_most: 5m        # not more often than every 5m (a run always follows)
  idle: 2m           # once no agent turn runs and nothing changed for 2m
  force: [effort.finished]   # these run at once, past the rest
```

Any combination; without them every event runs at once, as before.
`force` needs at least one of the others and names types in `on`.
Durations are `30s` / `5m` / `2h` (`parse_duration`). Checked at load
(`collectors::parse_trigger`); `Pacing` on `Trigger::On`.

- **Deferring.** The `collector.triggers` consumer runs the collectors an
  event triggers that don't defer it (`FactCollector::defers`, the passes'
  `select`), and records the rest in `pending_run` (V3; one row per
  collector: the latest `event_seq`, `since`, `touched`), readable as
  `v_pending_run` — a view showing a collector's result can say
  it's updating while its row is there.
- **Core's jobs too.** Change analysis's deep stage is paced the same
  way, as `pending_run` rows owned by `core` (semantic-layer.md "Change
  analysis"); `run_due` runs them through `refresh_change` (a `Busy` one
  waits for the next pass).
- **Running** (`crate::pacing`, spawned at boot, every 5 s): `run_due(now)`
  runs each pending collector once `due()` holds — settled since
  `touched`, `at_most` since its last `collector_run`, idle (`idle_for`:
  no open `agent_turn`, and nothing since the last take that recorded
  files or agent event) — for its latest event, then clears it unless a
  newer event deferred it again meanwhile. A gone or no-longer-paced
  collector, or an expired event, drops its row.
- **Nothing skipped.** A snapshot collector's deferred run
  (`MetricsService::run_paced`) runs over the stream's latest snapshot
  with every file recorded since its last done capture
  (`list_span_files`) — what running on each snapshot between would have
  scanned — so pacing never loses a file.

### Exec fact collectors need `permissions.process`

Only an own extension's (the project's) fact collectors may be `runtime: exec`
(any other extension's is starlark or jaq, and a fact collector gets no `env` /
`credentials` / `network`). One loads only when its extension declares
`permissions.process` (and the `network` / `secrets` it uses), which a person
approved when they deployed it on this machine, like any exec collector; it
runs from the extension's immutable deployment, so the content that runs is
the content that was approved (checked in `MetricsService::fact_runner`).

Own-extension exec fact collectors run with owned process cleanup and live-owner
checks. Relative code and helpers are read from the deployment;
`OXPLOW_PROJECT_DIR` names the checkout for their data access. See
[collection.md](./collection.md) "Authority for external report parsers".

Contained fact scripts also pin the loaded text to the current entry for each
run, including previews. A new deploy cannot authorize older cached code;
service calls and final results recheck the pin. Previews use the collector's
ordinary declaration and grant in pure mode, so a fixture cannot bypass model
or tree bounds or spend a model key.

- **Background runs** (snapshot / event triggers) log "fact collector: not run"
  and record a failed run (`collector_run` + `collector.synced`).
- **Explicit runs** (`run_collector_by_key`, the `oxplow.collector.sync` command)
  return the reason as the command's error.
- **Approving.** Deploying the extension on the Extensions page; the deploy
  asks a person when it adds `process`. See
  architecture.md → "A repo's config never runs a program without a deployed
  permission".

### Producers — facts on the capture spine (the ONLY write since T-E2)

Each producer writes atomic facts through `record_facts` (a capture + the
facts). The legacy V38 sample/finding/run/definition writes were removed in
T-E2 (tsk49); the change loop announces `MetricSamplesChanged` for what they
wrote (P7.B1) — a producer emits nothing itself.

**Collection gate (tsk31).** Before writing, each base-data producer checks
`fact_store.measure_has_active_spec(<measure>)` and skips when no *enabled* metric
consumes that measure (the spec table = the enabled set after the generation's
reconcile). So disabling every metric over a measure stops its collection:
`oxplow.tokens` (all `agent.tokens.*` off), `oxplow.test_case` (all
`oxplow.tests.*` off), `oxplow.lint_hit` (both `oxplow.analysis.*` off),
`oxplow.coverage`, `oxplow.turn`, `oxplow.nudge`, `oxplow.cycle_time` /
`oxplow.work_item_effort`. **Tests keep their run record** even when the metric is off
— a measured run whose `oxplow.tests.*` are all disabled records under the
record-only `test-run` producer (no metric facts) so effort-review still sees the
run. Built-in code collectors need no gate — `fact_collectors()` already elides
one whose metric isn't enabled (it never runs). Test fixtures that exercise a producer must seed the
producer specs (as boot does) or the gate stays closed.

Landed:

| producer | where | facts |
|---|---|---|
| tokens — OTEL (tsk22, P10.M2) | the `token_usage.otlp` consumer (`token_usage.rs::record_reported`) over `agent.tokens.reported`, which the control-plane `POST /v1/metrics` OTLP receiver logs (`otlp_ingest.rs`) | PER-KIND facts on `oxplow.tokens` (one input + one output per model, sliced by the `oxplow.token_kind` dim), producer `otel-tokens`, one capture per export keyed by its event (`otel-tokens:<event id>`; a retransmit logs no second event). The capture carries the event's anchors: the thread (the exporter's session bearer), its stream, the turn the export measured and the effort open during it. `agent.tokens.total` sums both kinds; input/output specs filter by `token_kind`. **Source of the token facts** — see [OTEL token tracking](#otel-token-tracking-tsk22) |
| prompt-cache tokens (tsk73) | same ingest, same capture | Cache kinds (Claude `cacheRead`/`cacheCreation`) land on **`oxplow.cache_tokens`** — a SEPARATE measure, because `agent.tokens.total` is an unfiltered sum over `oxplow.tokens` and cache facts there would silently change its meaning. Plus one per-model **`oxplow.cache_usage`** ratio fact per export: `num = cache_read`, `den = input + cache_read + cache_creation` (prompt-side; output can't be cached) — non-additive, so the cross-time collapse is the cumulative Σn/Σd hit ratio (`agent.tokens.cache_hit_pct`). An export with NO cache telemetry emits no ratio fact (an agent that doesn't report cache reads as "no data", not 0%). **Token-denominated only — never dollars**: the API returns token counts; a locally maintained price table is invalid by construction (Claude Code's OTEL `cost.usage` *estimate* would be the only defensible future dollar source, not ingested today) |
| effort token spend (tsk73) | `effort_service.rs::project_effort_lifecycle_metrics` (a close-time sub-producer) | one **`oxplow.effort_tokens`** fact per closed effort: Σ of ALL token kinds from its effort-stamped otel captures (num=value/den=1, non-additive → `task.tokens` reads the MEAN tokens per close — the cost of a unit of work, in tokens). No fact when the effort has no token captures (unmetered ≠ zero) |
| wasted tokens (tsk77) | close-time producer + `collection.rs::record_token_waste_for_reverts` (fires on any landed commit incl. `git revert`, via `detect_git_revert` — revert never says "commit") | **`oxplow.token_waste`** is an append-only ratio measure with two writers: a metered CLOSE emits (num 0, den = the effort's spend, value 0) — rides inside the effort_tokens gate since the denominator IS that spend — and a detected revert emits (num = spend, den 0, value = spend) for the ONE closed effort whose window contains the reverted commit (`This reverts commit <sha>` trailers in HEAD; 0/ambiguous candidates → no attribution; idempotency key `token-waste:<effort>` → one waste fact per effort ever; commit times are seconds-granular so containment spans the whole second). `task.tokens.wasted` = SUM over values (closes are 0); `task.tokens.wasted_pct` = ratio Σn/Σd = wasted ÷ all metered spend. V1 is coarse: one reverted commit flags the effort's FULL spend. Pre-V61 closes never entered the denominator |
| effort steering (tsk76) | same close-time producer | one **`oxplow.effort_steering`** fact per closed effort (num=value/den=1, non-additive → `task.steering` reads the MEAN per close — the autonomy number, lower = more autonomous): user prompt submissions (`agent_turn` rows opened in the effort window, newest-1000 scan) + Stop-hook nudges (Σ of the effort's `oxplow.nudge` facts) + non-`agent`-authored comment threads opened in the effort's thread during the window. **Zero IS emitted** — a fully autonomous effort is real data. Interrupts are NOT counted (nothing records them yet). Needs `with_steering_sources` (agent-turn + comment stores) wired, as boot does |
| effort test outcome + time-to-green | oxplow-foundation's **`oxplow.effort_test_outcome` collector** (`collectors/effort/test_outcome.star`, `trigger: { on: [effort.finished] }`; core computes none of it) | Four **`oxplow.effort_test_outcome`** facts per finished effort, sliced by `oxplow.tests_stat` — `at_close` (failed count of the last run = quality gate), `peak` (max failed in any run), `distinct_failed` (distinct cases red in ≥1 run; asserted failures have no identity, so it never reports fewer than `peak`), `red_runs` (runs with ≥1 failure) — and one **`oxplow.effort_time_to_green`**: ms from the FIRST red run to the first green after it, only when that transition exists (never red or never recovered is "no data", not a zero). `effort.time_to_green_ms` reads the mean. All `num=value, den=1` (non-additive → the mean per effort), subject the effort ref, the capture stamped with the effort, thread and branch (`oxplow.tests.*` slice by `oxplow.branch`). **Its `input:`** is the effort's measured runs — `tests` captures (`v_capture`, not the record-only `test-run`) joined to `v_test_run` and their failed `v_test_case` rows, oldest first; a run that listed no cases and asserted `failed` counts that many anonymous failures; a run with no failed case (a green repeat that wrote none) still counts. These "within-effort" aggregates are **not expressible** as a metric spec (the engine's temporal collapse is only sum/last/Σn÷Σd), so a collector materializes them; no result means no facts. Pinned by `foundation_collectors_tests.rs` (the script over row fixtures) and `bundled_extensions::tests::the_test_outcome_collector_writes_what_the_producer_did` |
| turns — transcript (tsk22) | `token_usage.rs::record_token_metrics`, from `on_stop` | a `oxplow.turn` fact per model per Stop (turn COUNT = genuine user prompts). The transcript path **no longer projects `oxplow.tokens`** (OTEL owns those); it still records the per-turn `agent_token_usage` rows (with prompt text OTEL lacks). The Claude harness's transcript parse counts each `message.id` once (tsk22), which removed the ~2–3× overcount from Claude repeating a message's `usage` on every content-block line |
| effort lifecycle (T-B) | `effort_service.rs::project_effort_lifecycle_metrics` | one `oxplow.cycle_time` fact per close (subject=effort) + one `oxplow.work_item_effort` fact (subject=work_item, the effort's work-item ref; the efforts-so-far redo signal, none while unlinked); both carry `numerator=value, denominator=1` (the measures are non-additive per V47, so Σn/Σd across time = the MEAN across closes, tsk42); capture **stamps `effort_id`** (unambiguous — this producer knows the exact effort). (The effort's test outcome and time-to-green are not written here: oxplow-foundation's `oxplow.effort_test_outcome` collector records them when the effort finishes.) |
| nudges (T-B) | `collection.rs::project_nudge_metric` | one `oxplow.nudge` event fact per fired nudge (value 1, subject=the nudge kind) — the `agent.nudges.fired` spec is `Sum(oxplow.nudge)` |

**Every close, one place.** `project_effort_lifecycle_metrics` runs from the
effort-lifecycle pump consumer on `effort.closed` — however the effort closed
(the policy, `oxplow.effort.close`, a thread closing, a stream archived) — once per
effort (it stops at an existing `effort-lifecycle` capture).
| lint hits | `collection.rs::mirror_analysis_metrics` | one `oxplow.lint_hit` fact per finding (severity/rule/detail columns + file location) |
| coverage | `collection.rs::observe_coverage` | one `oxplow.coverage` fact per file (value=line-%, num/den=covered/instrumented → engine re-derives Σcov/Σinstr). **Branch + function coverage (tsk123)** ride the SAME capture as extra per-file facts on `oxplow.coverage.branch` / `oxplow.coverage.function` (num/den=hit/found), emitted only for files whose report carried the counts (`*_found > 0`) and only when their spec is enabled (per-measure gate via `active_coverage_measure`). Specs: `oxplow.coverage.branch_pct` / `oxplow.coverage.function_pct` (ratio %, higher-better). **Untested files (tsk124)** is a read-only spec `oxplow.coverage.untested_files` — a `count` over `oxplow.coverage` filtered `max_value: 0` (the new upper-bound `FactFilter` field, cube-ineligible like `min_value`), `findings` display so the drill-in lists which files, lower-better — no new collection |
| test cases | `collection.rs::record_test_run` → `SqliteFactStore::record_test_run` (tsk733) | **change-only** per-case facts on `oxplow.test_case` (status as the `oxplow.status` dim, + `oxplow.test_suite`) and `oxplow.test_duration`: every **failure**, every run; a pass or skip only when the test is new on the branch or changed status; a duration when it moved more than `DURATION_MOVE_RATIO` (50%) **and** `DURATION_MOVE_MIN_MS` (20 ms) from the last one written (constants, not settings — a compression tolerance that changes no pass/fail number). The per-subject fold carries an unchanged test's last fact forward, so every `oxplow.tests.*` number is the same as recording all cases (durations within the tolerance). In the same transaction it upserts each case's `test_case_stat` row (`v_test_case_stat`: last status and duration, runs, failures, flips, last failed / passed, max and mean duration) — where per-test history lives. A replayed run (same idempotency key) changes nothing. Measured on sample usage (tsk732): ~2.6% of per-case results are written. The effort test-outcome collector takes the effort's runs from its run records, not its facts, since an all-green repeat run writes none. MCP-asserted counts (no report) synthesize status-sliced facts (no case identity). A report-less, count-less run records its capture under the **`test-run`** producer — a run RECORD, not a measurement: an empty `tests` capture would read as "found 0 tests" to the zero-fill/currency logic and collapse the semi-additive `oxplow.tests.*` timeline |
| duplication | oxplow-foundation's collector `oxplow.duplicate_lines` — whole tree, after ref moves (paced: one scan of the latest tree once they settle for 60 s, at most every 15 min); the change analyzer's scoped scans record findings but no facts (tsk365) | one `oxplow.duplicate_lines` fact per side of each duplicate block (value=line count, subject=`block:path:start-end`); a `full` capture on the snapshot's stream. A zero-hit scan still writes its EMPTY capture (tsk44 currency) — else the last non-empty scan's blocks stay "current" forever |
| foundation code collectors | `metrics_service.rs::run_one_collector` → `record_collector_facts` (tsk23) | oxplow-foundation's code collectors return `facts`: one fact **per function** on `oxplow.complexity` (high_complexity_fns) / `oxplow.fn_length` (long_functions) / `oxplow.parameter_count` (fn_count), and one per marker on `oxplow.todo` (todos) — the raw grain, for **every** item, not just the offenders |
| per-language idiom collectors | same path (tsk30) | the ~10 idiom collectors (`oxplow.rust.unsafe_blocks`, `oxplow.ts.any_usage`, `oxplow.csharp.empty_catch`, …) emit one **per-file** `oxplow.ast_hit` fact (value=the file's count, `rule`=the idiom slug, dims carrying the conformed `oxplow.language`); the metric is a `Sum(oxplow.ast_hit)` spec filtered by `dim_eq(oxplow.rule, <slug>)` (the foundation's `metrics:` entries) |

#### OTEL token tracking (tsk22)

**A Codex collab agent's tokens are not counted.** Its work is a subagent of
the parent's session (`agent-model.md` "Collab agents are subagents"), but
its tokens are its own thread's `thread/tokenUsage/updated`, which the
observer ignores; the parent turn's counts are the parent thread's alone.
Per-subagent token facts are not recorded.

Token facts come from **OpenTelemetry**, not transcript parsing. The old
Stop-hook transcript parse overcounted ~2–3× (Claude writes one JSONL line per
content block and repeats the message's cumulative `usage` on each; the parser
summed every assistant line) and was Claude-only + format-fragile.

- **Receiver:** the control plane hosts `POST /v1/metrics`
  (`oxplow-control-plane/src/lib.rs::handle_otlp_metrics`), behind the same
  session-bearer auth as `/hook`: the export comes from the session its
  bearer stands for (and that session's thread and stream), and the raw
  body goes to `OtlpIngestService::ingest`. Always
  answers a 200 OTLP ack (best-effort side-band; a non-2xx would make the
  exporter retry-storm).
- **An event, then facts (P10.M2):** `otlp_ingest.rs` decodes the body
  (`otlp_tokens::decode_token_export`) and logs **one
  `agent.tokens.reported@2 { thread, counts: [{model, kind, value}],
  window_end? }`** in one transaction — deduped by a hash of the body, so an
  SDK retransmit logs nothing; an export for a thread that isn't there
  (deleted, or never) logs nothing, as a hook for one doesn't. The body is
  untrusted (tsk925): a negative count is dropped. It is
  anchored to the thread, its stream, **the
  turn the export measured**, and the effort open during that turn (tsk900).
  A Claude export is stamped with when it was *collected* (delta
  temporality: its points cover `start_time_unix_nano..time_unix_nano`, since
  the last collection), so its window can reach into the turn after the one
  that spent the tokens: the export goes to the turn whose span overlaps the
  window most, the earlier on a tie (`agent_stores::turn_for_window_tx`); with
  none overlapping, the newest started before the window's end
  (`turn_at_tx`). The effort is
  the one whose span overlaps that turn's, when exactly one does
  (`effort_during_turn_tx`) — not the one open as the export arrives. An
  export that doesn't say when is the open turn's and the open effort's.
  The async `token_usage.otlp` consumer
  (`TokenUsageService::record_reported`) is the only writer of the token facts:
  one `otel-tokens` capture per event carrying its turn and effort, keyed
  `otel-tokens:<event id>`, so a redelivery counts nothing twice.
- **Decode + map:** `oxplow-app/src/otlp_tokens.rs` decodes the OTLP protobuf
  (`opentelemetry-proto` crate) into neutral `OtlpRecord`s (metric data
  points; `oxplow_domain::agent::observe`) and asks every
  registered harness for its `TokenReading`s (`AgentHarness::token_readings`,
  in `crates/oxplow-harnesses`): the metric names are each
  harness's own, and asking only the session's harness would cost a database
  read per export. Pure + unit-tested:
  - **Claude** — `claude_code.token.usage` **counter** (delta temporality → each
    export is the increment), `type ∈ {input,output,cacheRead,cacheCreation}`
    (the cache kinds on their own measure, tsk73);
  - **Codex** exports nothing: its tokens come from its thread's
    `tokenUsage` and ride the turn's Stop ([agent-model.md](./agent-model.md)
    "Codex: its TUI on a thread oxplow observes").
- **Launch wiring:** per-agent, injected by each harness's `launch`:
  - **Claude** (`otel_env` in its harness, env — Claude has OTEL env support):
    `CLAUDE_CODE_ENABLE_TELEMETRY=1`, `OTEL_METRICS_EXPORTER=otlp`,
    `http/protobuf`, `OTEL_EXPORTER_OTLP_ENDPOINT` = the control-plane
    `otlp_base_url` (base; SDK appends `/v1/metrics`) threaded via
    `PluginRuntime`, + the session's bearer.
  - **opencode** is not auto-instrumented (a user's own OTEL plugin pointed at
    the receiver still works).
  > **Diagnostic (tsk25):** set `OXPLOW_OTLP_DEBUG=<file>` before launching
  > oxplow — the receiver appends a decoded dump of every OTLP export (metric
  > names + attributes + values) to that file
  > (`otlp_tokens::summarize_metrics_request`). Off by default.
  See `.context/agent-model.md`.

**Fact-attribution spine — `metric_capture.effort_id`.** The read-side
effort attribution resolves an effort's facts from *its captures*
(`captures_for_effort`). The effort-scoped producers stamp
`capture.effort_id` at write time with
`CollectionService::resolve_owner(thread, anchored)`: the causing tool
event's effort anchor, else the thread's open effort, else null. Stamped
by: **tokens/turns** (`token_usage.rs`, the effort resolved once in
`on_stop`), **tests / lint-hits / coverage / nudges** (`collection.rs`),
and **effort-lifecycle** (`effort_service.rs`, which knows its exact
effort). An effort that adopts a turn restamps its captures. The
**snapshot fact-collector** captures are deliberately NOT stamped —
they're whole-tree scans whose baseline predates the effort, so the File
family attributes them by *the effort's files × time window*, not by
`effort_id`.

**Code-metric unbake (tsk23) — the keystone, and the one non-mechanical producer.**
A code scan's output used to be baked `samples` (headline) + `findings`
(offenders drill-in); tsk23 added a `facts` channel, and P7.B3 removed the
other two. A fact collector now returns only `{"facts": [CollectedFact { measure,
value, subject?, path?, line?, rule?, num?, den?, dims? }]}` — measure-bound
atomics (`facts_of` refuses any other shape). `record_collector_facts` resolves each
fact's `measure` against the catalog (**declare-to-collect**, decision #4: a fact on
an undefined measure is dropped with a `tracing::warn!`, never silently written) and
writes the resolvable facts under one capture — the **only** output now (facts-only,
T-C3b). A ZERO-fact run still writes its (empty) capture — "this scan ran and found
nothing" is the record the engine zero-fills a series from, so a count metric drops
back to zero after the last offender is fixed (tsk44; the analysis ingest records
its capture for a clean report the same way). The count-over-threshold headline is the **spec** (the foundation metric's `metrics:` entry),
and the equivalence test `code_collector_facts_reaggregate_to_the_expected_headline`
pins `engine.headline_for_spec(spec) == the expected baked total` for every bundled
code metric — the proof the inversion is faithful. (Strict `> N` in the old baked count
equals `min_value = N+1` on the integer complexity/length measures.) The 4 code
scripts are unbaked (no `tree:.`/`file:` samples); the baked write path is gone.

**Per-language idiom collectors (tsk30).** The same pattern extends to the ~10
per-language AST idiom collectors, but they don't have a natural per-subject measure —
so they share **one** generic measure `oxplow.ast_hit` (a per-file idiom
occurrence) and are told apart by the `oxplow.rule` dimension (the idiom slug,
carried on the fact's `rule` column via `CollectedFact.rule`). Each collector records
one per-file `ast_hit` fact (value=that file's count, `rule`=its slug); each metric is a `Sum(oxplow.ast_hit)` spec filtered by
`dim_eq(oxplow.rule, <slug>)` (the foundation's `metrics:` entries). Idioms
sharing the measure never collide because every spec read applies the rule filter
**before** it aggregates. `per_language_collector_facts_reaggregate_to_the_baked_headline`
pins each spec's `Sum` to its baked headline. The `<slug>` in the script and the
spec MUST match (the equivalence test catches a drift → spec count 0 ≠ baked).

Wired into `Services` as `fact_store: Arc<SqliteFactStore>` +
`metric_engine: MetricEngine`; `EffortService`/`CollectionService`/
`TokenUsageService` carry the fact store; the duplication write lives in the rpc
layer (`svc.fact_store`).

**Producer specs (T-B).** Each always-on producer metric now has a `metric_spec`
(a `metrics:` entry of oxplow-foundation's manifest under `# what oxplow
records itself`, seeded by the generation like any extension's) — the aggregation it *is* over the measure its producer
emits facts on, with conformed dims (not extra measures) distinguishing variants:
token in/out slice `oxplow.tokens` by `oxplow.token_kind`; tests slice
`oxplow.test_case` by `oxplow.status`; analysis filters `oxplow.lint_hit` by
severity; coverage is a `ratio` over `oxplow.coverage`. Each entry holds its
`sourceMeasure`, `aggregation` and `filter`; the producers' Rust only records
the facts. Equivalence tests pin
each spec's engine headline to the baked total (tokens, tests). New V46 measures:
`oxplow.turn`, `oxplow.task_effort` (now `oxplow.work_item_effort`, V23), `oxplow.nudge` (the producers with no prior
measure home); new dim `oxplow.token_kind`.

**Decision reversed — nudges are now IN the substrate (T-B, was tsk24).** The
earlier call kept the advisory nudges (report-less, coverage-target,
threshold-crossing) out of the substrate. T-B reverses it: with the
producer-spec layer in place, adding an `oxplow.nudge` event measure + one fact
per fired nudge is cheap and makes the `agent.nudges.fired` operational metric a
first-class spec like every other. The nudge rows in `agent_nudge` stay the
authoritative store; the fact is the analytics grain.

### Read and write surface (P4.8)

**Reads are SQL.** Agents and the desktop read metrics through `oxplow_query_sql`
— `metric_grid(bucket[, dim])` with `MEASURE('<key>')` for series (the
engine computes each one; see `.context/semantic-layer.md` "Metrics in
SQL"), and the views `v_metric_spec`, `v_metric_catalog`, `v_measure`,
`v_dimension`, `v_capture` and `v_fact` for definitions and raw facts, and
`metric_findings(key[, capture])` for the located items behind a metric as
they stand now. The engine's by-dimension rollup (`rollup_for_spec`,
`compute_rollup`, `RollupRow`) and the never-wired formula evaluator
(`evaluate_formula`, `BinaryOp`) are deleted: a slice is
`metric_grid(bucket, dim)` (tsk516). The
metric-specific MCP reads (`list_metric_definitions`, `list_metric_samples`,
`get_metric_summary`, `metric_breakdown`, `list_metric_findings`,
`list_measures`, `list_dimensions`, `list_facts`, `metric_series`,
`metric_rollup`) are deleted, and the parity manifest holds no metric read
(`no_surface_carries_a_metric_read`).

**The cube stays** (P4.4): at 7.1M facts a cold read took 68.3 s from
facts and 1.6 s from the cube; the measurement is in
[performance.md](./performance.md) "The cube decision".

**Writes are `metric.*` commands** (`crates/oxplow-app/src/commands/metric.rs`,
reached through `run_command`, audited like every command):
- `oxplow.metric.enable { keys, enabled }` — `Tx`, through `oxplow.config.set`'s core.
- `oxplow.metric.record { key, value, subject?, dims?, stream? }` — `Tx`: an
  asserted fact (below) written in the bus transaction with its audit
  (`fact_store::record_facts_tx`); after commit it clears the fact memo
  (the change loop announces `MetricSamplesChanged` for the measure).
- `oxplow.metric.rebuild { force }` — `BestEffort` over
  `MetricsService::rebuild_baseline`, the whole-tree baseline boot runs. Its
  `BaselineReport` carries `excluded: [{ path, reason }]`: the snapshot files
  the collectors were not given, `oversize` (over `snapshotMaxFileBytes`, or
  stored by stat only), `binary` (a NUL byte) or `unreadable`
  (`file_map_from_rows` builds a `Corpus` of the text and what it left out; a
  sweep logs a warning naming them and `SweepReport.excluded` carries them).
  **An excluded path keeps its last good facts.** The exclusions are stored
  with each capture (`metric_capture_exclusion`, read as `v_capture_exclusion`:
  `capture_id`, `producer`, `path`, `reason`, `unchecked`), and the per-path
  fold (`latest_tree_facts`, `v_tree_fact`, `scanned_paths_for_captures`) does
  not count an excluded path as restated by that capture, so a file that grows
  past `snapshotMaxFileBytes` (or turns binary or unreadable) no longer reads
  as scanned clean with its facts zeroed: the numbers it last had stay, and
  `unchecked = 1` says no later capture of that producer has scanned it since
  (a later one that also leaves it out is its own row; a full baseline that
  reads it, or a delta whose snapshot has it, clears it). The baseline prune
  (`prune_dominated_tree_captures`) keeps the older captures that hold such a
  file's last good facts. The metric's detail page lists the unchecked files
  under its chart (`UNCHECKED_FILES_SQL`), and a lens or `oxplow_query_sql` can join
  `v_capture_exclusion` with `v_fact`/`v_capture`. A capture before this was
  recorded has no exclusions. Oversize files can also be listed from
  `v_snapshot_file` (`storage`).
- `oxplow extension check` against a project database resolves a metric the
  extension under check declares even when the app has not loaded it yet
  (`MetricEngine::with_declared`, `declared_metric_specs`): the store's spec
  wins, else the extension's own, so a lens reading `metric_findings()` or
  `metric_grid()` of it dry-runs.
- `oxplow.metric.scaffold { name, language?, glob?, origin? }` — a `Read`: the
  files of a code metric extension (`{ name, dir, files: [{ path, content }] }`),
  which the agent writes with its own tools; then `extension check` and
  `extension test`. Refused for an existing folder, a bad or reserved name,
  or a language with no starter sample.

Running one fact collector now is **`oxplow.collector.sync { collector }`** (the
one manual run for every collector; `metric.run` is deleted) — it reaches
`MetricsService::run_collector_by_key` and returns the `facts` count (see
"Producers" below and [commands.md](./commands.md)).

A stream defaults to the caller's, else the primary; an agent may only
name its own.

### Catalog authoring surface (`measures:` / `dimensions:` — workstream E)

Custom fact types and slice axes are **pluggable**, namespaced exactly like
metrics (`oxplow.*` reserved to oxplow-foundation). They are declared in an
extension's `extension.yaml` (the project's own: `.oxplow/extensions/project/`)
or the person's global files; `.oxplow/project.yaml` refuses `measures:` and
`dimensions:`. `crates/oxplow-config/src/lib.rs`:

```yaml
measures:                          # custom fact TYPES a collector may emit
  - key: acme.api_latency
    unit: ms
    subjectKind: endpoint
    temporalSemantics: non-additive   # additive | semi-additive | non-additive
dimensions:                        # custom conformed slice axes
  - key: acme.license
    valueType: categorical            # categorical | numeric | temporal | entity-ref
    vocabulary: [MIT, Apache-2.0]     # optional controlled value set
    promote: true                     # request a generated column + index (see below)
```

- **Definition-only** (no `use:`/`key:` split like metrics — you declare a fact
  type / axis, you don't "enable" one). `validate_measures`/`validate_dimensions`
  mirror `validate_metrics` (namespacing, `oxplow.*` reserved, per-key
  uniqueness, enum checks), each taking an `oxplow_config::CatalogLayer`
  (`Project`, `Global`, `Extension(ns)`) that says where the block is written.
  `resolve_measures`/`resolve_dimensions` merge the **global** files and each
  **extension** (extension over global) into flat `Resolved*`.
- **Global scope** = `<global_config_dir>/{measures,dimensions}/*.yaml`
  (`load_global_measure_entries`/`load_global_dimension_entries`); auto-active in
  every project (unlike a global metric, which needs a project `use:`). The four
  `load_global_*_entries` loaders share one generic `load_global_entries` helper
  (tsk17 — they differ only in the doc field + validator).
- **Read-path caching (tsk17):** `resolved_specs`/`fact_collectors` run on
  **every** snapshot event, so the three global YAML dirs are loaded once into a
  `MetricsService.global_catalog` (`Arc<Mutex<Option<GlobalCatalog>>>`) and
  served from cache (`with_global_catalog`); `reseed` clears it on every
  config change (`config.metrics`) and extension change before reseeding (an external edit to a global file
  needs a config change to refresh). Project config (`use:` entries) stays read fresh from the in-memory `RwLock`.
  `with_global_dir` forks a fresh cache (dir changed). Two more per-read memos:
  `effort_metric_deltas` loads each measure's history **once** across the
  File-family specs sharing it (a per-call `fact_cache`), and `dim_value` parses
  a fact's `dims_json` **once** per lookup (`parse_dims` + `dim_from_map`).
- **Boot seeding:** The metric catalog is published by the generation at boot and on
  every reseed — a config change (`config.metrics`) or extension change (the
  catalog's signal) — (beside `seed_definitions`), upserting resolved
  measures/dimensions into the `measure`/`dimension` tables. `MetricsService`
  holds a `fact_store` via `.with_fact_store()`. Metric specs seed in two
  passes: EVERY resolved spec — oxplow-foundation's producer, entity and
  code metrics among them — carrying the catalog default target plus the
  project's target/warnAt/failAt overrides (from a `use:` entry). Nothing in
  the metric catalog has the `built-in` scope: oxplow's own measures,
  dimensions and metrics are the foundation's declarations (the V66
  migration restated the seeded measure/dimension rows as the foundation's,
  keeping them so their facts survive).
  `resolve_metrics` itself is silent: it runs on every event (through
  `fact_collectors`), so a `use:` naming nothing is reported by
  `unknown_uses` once per generation (boot, config or extension
  change), never per call (tsk1076).
- **Adding one** is an edit to the project's extension (`extension.yaml`; a global
  one is a file under the global dir). The old `scaffold_measure` /
  `scaffold_dimension` writers had no caller and were deleted (P4.8).
- **`promote`** now persists onto the row: the generation threads the resolved
  dimension's `promote` into `NewDimension.promoted`, so `dimension.promoted`
  reflects the config (it was previously parsed but dropped at seed). Still
  **inert** downstream (see tsk28): the engine loads all facts and filters
  in-app, so the requested generated column + index bites nothing until reads
  go DB-side. Recorded, not yet acted on.

**Not yet done:** a **Dimensions catalog** UI page; `promote_dimension` teeth
(tsk28); unbaking the per-language idiom scripts (still emit dead `tree:.`/
`file:` samples, harmless — `run_one_collector` ignores them); formula-spec wiring
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
  `language`, `scope` (global|`extension:<name>`), `dimensions_json`,
  `target`/`warn_at`/`fail_at`. Upserted by `key`.
- **`metric_dimension`** — conformed-dimension catalog (seeded: time, stream,
  thread, effort, vcs_rev, branch, subject, model, agent, language,
  severity, status). Shared meaning across metrics → cross-metric drill-across.
- **`metric_subject`** — subject hierarchy (file→module→package→repo) for
  roll-ups. Declared, not yet exercised.
- **`metric_run`** — a compute event (generalizes `code_quality_scan`): producer,
  status, trigger, provenance/source, snapshot/git/**branch**. One run can feed
  many metrics. Raw events have **no run** (`run_id` NULL).
- **`metric_sample`** — the durable scalar fact (the BI grain). `value`
  (+ `numerator`/`denominator` for ratios so roll-ups RE-AGGREGATE correctly),
  `captured_at` (the spine) + `closest_vcs_rev` + `branch`, optional
  `subject` (a ref) / `path`/`line`, `dims_json`, `provenance`/`source`.
- **`metric_finding`** — located detail for the `findings` kind (generalizes
  `code_quality_finding`): path/line, kind, severity, rule, message, value.

### Time-primary, effort-as-overlay (the key invariant)

A sample carries **NO `effort_id` FK**. It's anchored by `captured_at` +
`closest_vcs_rev`. Efforts (and later commits/releases) are **time-range
overlays** read from `effort` (`started_at`/`ended_at`) — so:
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

- **the effort's files × time** — code-metric collectors are snapshot scans, so their captures are
  **not** effort-stamped; the File family reads them by the effort's paths (claimed or observed) + capture time,
  scoped to the effort's stream. Exception: an `{ on: [effort.finished] }` collector run KNOWS
  its producing effort and stamps the capture (tsk43 — `CollectorRunContext.effort_id`),
  so its just-after-close capture still counts as the effort's "current".
- **`metric_capture.effort_id`** — the run/operational producers stamp the owning
  effort at ingest (`resolve_owner`), so an effort's run + token +
  nudge facts are exactly `captures_for_effort(effort_id)` → `facts_for_captures`.

| family | how the delta is computed |
|---|---|
| **File** — snapshot-scan metric (`display_kind` ∈ {`gauge`, `findings`}, a source measure, no formula, non-producer, non-operational — includes the `static-quality` built-in code metrics, whose captures are never effort-stamped; tsk43) | Σ over the effort's **files** (`effort_file`, claimed or observed) of `(current − baseline)`, each fact contributing per the spec's **aggregation** (`count` ⇒ 1 per offender — matching the Metrics page — else the fact value); facts are scoped to the effort's **stream** (worktree). Baseline capture = latest before the effort start; current = latest at/before the effort end (newest when open; a capture STAMPED with this effort — an `effort.finished` collector run — also counts). A CLOSED effort with no in-window capture yields no row (never a post-close capture, never a fabricated drop-to-zero). An effort file absent from a capture = 0 (sparse emission → a drop-to-zero is seen), and the producers' EMPTY zero-hit captures are spliced into the timeline so a scan that found nothing is eligible as baseline/current (tsk44). **No files, or repo-scalar facts with no path** → the repo-wide before→after fallback. `file_delta_from_facts` picks the two captures from capture metadata (the producers of the spec's slice, `producers_of_slice`) and loads only those captures' facts (`facts_for_captures`); a per-path measure's repo total at a capture is the metric engine's cube-backed series point, never a replay of the history. |
| **Run** — tests (category `testing`) + the `oxplow.analysis.*` producer pair | before→after (or `sum` flow) over `aggregate_series` of the facts of the effort's OWN captures (`facts_for_captures(measure, captures_for_effort)`). Analysis is classified Run by its `oxplow.analysis.` key (its facts arrive on effort-stamped run-ingest captures), so it never reaches the File branch (the tsk272 guard) |
| **Window** — operational (`agent.*`/`effort.*`/`task.*`) + formula/event specs | identical read to Run; a distinct family to document what it holds. `effort_stamped_delta` serves both |
| **Coverage** (category `coverage`) | effort-relative: for each of the effort's coverage run CAPTURES (`captures_for_effort`, trigger `on-report` — the capture is the run), `coverage_delta_for_spec` derives the **diff-coverage** (`diff_coverage_for_effort`, start snapshot vs the capture's own snapshot — collection.md) from the capture's ABSOLUTE per-file **line-sets** (`metric_capture.detail_json`, the `coverage-detail` envelope), then before→after over the derived sequence. The coverage FACTS carry num/den counts; the line-sets live only in the detail envelope |

The family is chosen by **one classifier** — `classify_effort_attribution(spec)
→ EffortAttributionFamily` (`crates/oxplow-app/src/attribution.rs`).
`effort_metric_deltas` `match`es on it; adding a fact-kind is one variant + one
match arm. A formula spec (no source measure) falls through to Window and no-ops.

### Run attribution grain — the causing tool call, not the clock

> **P3 (tsk476):** a run the `collection` reactor records carries the tool
> event that ran it: the capture is keyed `test-run:<event id>` (a
> redelivery records nothing new), the effort the command ran in owns it
> (the event's effort anchor), and its
> `test.run.recorded` is anchored to the turn.
>
> **P10 (tsk483):** the capture carries that turn too —
> `metric_capture.turn_id` (V157, SET NULL when the turn goes), exposed as
> `v_capture.turn_id` and `v_test_run.turn_id`. A producer stamps what it
> knows, never a guess, by **where the run came from** (`RunOrigin`,
> tsk923): a run, coverage, analysis or nudge capture the reactor saw
> takes its causing tool event's turn anchor however late it is recorded
> (`RunOrigin::Tool` — none when the event had none); one reported by a
> command (`oxplow.test.record_run`, `oxplow.collector.sync`) takes its actor's open
> turn **only when that actor is the thread's own agent**
> (`CollectionService::command_turn`) — a person's sync while the agent
> is mid-turn has none — and its `test.run.recorded` is anchored alike.
> A nudge row and its capture take the same turn (`RunOrigin::turn`). An
> agent's `oxplow.metric.record` capture carries its thread and open turn; a
> person's, neither. The per-turn
> `oxplow.turn` facts' capture takes the turn whose `agent.turn.ended`
> counted them. A capture no turn produced (a scheduled collector, a
> baseline scan) has none.

**The capture IS the run.** Agent-work runs — tests, coverage, analysis —
are **observe-always**: every run writes its `metric_capture` + facts whether
or not an effort is open, attributed by the `capture.effort_id` stamp (the
causing tool call's effort), never by time window — the clock can't tell two
threads' runs apart. All stamp `trigger='on-report'` (`RUN_TRIGGER`), and each
carries its verbatim payload in `metric_capture.detail_json` as the envelope
`{"kind": "test-detail"|"coverage-detail"|"analysis-detail", "payload": {…}}`.
`effort_observations_from_metrics` reads the effort's own run captures
(`captures_for_effort`, filtered to that trigger) and their envelopes; the
metric-delta read (above) joins `capture.effort_id` the same way.
**Coverage** is effort-relative (diff vs the effort's start snapshot), so it
observes the ABSOLUTE report always and DERIVES the effort diff with the
effort's evidence (`diff_coverage_for_effort` over the capture's
`coverage-detail` envelope, against the snapshot the capture measured) — a
run attributed after the close still yields a diff.

### Additivity

Ratio metrics (coverage %, pass rate) store `numerator`+`denominator`. Roll-ups
MUST re-aggregate from components (`aggregate_ratio` = Σnum/Σden), never naive-
AVG a percentage. Non-ratio metrics use `default_agg`.

## Branch tracking

Runs and samples record the **branch** they were captured on (`branch` column,
a conformed dimension), when applicable (NULL for detached HEAD / non-git /
operational metrics). Captured from `Vcs::head` in the code-fact producers
(`current_branch`); operational producers (tokens) leave it NULL.

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
| otel-tokens | `crates/oxplow-app/src/token_usage.rs` (the `token_usage.otlp` consumer of `agent.tokens.reported`, which the control-plane OTLP receiver logs through `otlp_ingest` — tsk22, tsk860) | per-model `agent.tokens.{input,output,total}` from Claude's `claude_code.token.usage` OTEL counter. The fact's subject is the model's ref, `ai_model:<name>` (`uri::build::ai_model_uri`; `subject_kind` is `ai_model`), read back with `id_of(.., AI_MODEL)`. Tokens only — no derived USD cost (rates move; a stale price table is worse than none). The transcript `on_stop` path now projects only `agent.turns` + the per-turn `agent_token_usage` prompt rows |
| effort-lifecycle | `crates/oxplow-app/src/effort_service.rs` (`project_effort_lifecycle_metrics`, run by the effort-lifecycle consumer on `effort.closed`) | derived `effort.cycle_time_ms` (close − start, subject=effort) + `work_item.efforts` (efforts-so-far on its work item, the redo-rate signal) from `effort`; branch captured when the stream has a worktree |
| nudges | `crates/oxplow-app/src/collection.rs` (`project_nudge_metric`, called from `persist_nudge` after a fired nudge records) | `agent.nudges.fired` (event kind, run-less; value 1, subject=the nudge `kind`) — an agent-activity signal |
| fact collectors | `crates/oxplow-app/src/metrics_service.rs` (`MetricsService`) — the fact engine. Seeds a `metric_spec` per resolved `metrics:` entry; runs each **fact collector** (`fact_collectors()` = the enabled extensions' fact collectors (oxplow-foundation's and the project's own included); a collector that metrics name runs only while one of those metrics is on) on its trigger: `on:` from the `collector.triggers` pump consumer (`run_snapshot_collectors` for `snapshot.taken` that recorded files, `run_effort_collectors` over the effort's end snapshot for `effort.finished`, `run_event_collectors` over the stream's latest snapshot otherwise), `every:` from the scheduler, `manual` and any explicit run through `oxplow.collector.sync` → `run_collector_by_key(owner, id, stream, source)` | one `fact` per `CollectedFact` the script returns (bound to a defined measure in the collector's `facts`), version/branch/snapshot-stamped, under one `metric_capture` (a failed capture on error), plus a `collector_run` row and a `collector.synced@2` event carrying the `facts` count. `facts_of` refuses any output but `{"facts": [...]}` |

> Navigation / activity (`page_visit`, `usage_event`) are **deliberately not
> projected** into the substrate: they're oxplow-usage telemetry (UI metadata),
> not code or agent-activity metrics, so they stay in their own tables.

Each producer: `upsert_definition` (idempotent) → `record_run` → `record_sample`(s);
the change loop announces `OxplowEvent::MetricSamplesChanged` for what landed.

> **The plan is for these to become bundled collectors** (jaq/Starlark/exec,
> registered via `with_builtins()`) so producers are *content*, not hardcoded
> Rust (tsk218). The hardcoded mirror helpers are the interim. The legacy
> `effort_observation` path has already been **dropped** (tsk215) — the substrate
> is the sole store.

> **Producer metrics are the foundation's declarations.** The Catalog lists
> them before any data because they are `metrics:` entries of
> oxplow-foundation's manifest (default on, a `enabled: false` marker turns
> one off), not Rust. Add or rename a producer metric there; the producers
> only record the facts. Coverage's red/green thresholds
> (`target`/`warnAt`/`failAt`) are on its entry.

## Read surface

> **Flipped (T-C2, tsk35 + T-C3a, tsk39):** the metric-key reads on BOTH the MCP
> and the IPC surface now read the fact substrate via `MetricEngine`
> spec-wrappers, NOT this V38 store. The paragraph below is the historical V38
> shape, kept for context; the current IPC wiring + types are in the **IPC**
> bullet.

- **Agents** read through `oxplow_query_sql` and change metrics through the
  `metric.*` commands (see "Read and write surface (P4.8)"). An asserted
  fact (`oxplow.metric.record`) lands on the metric's source measure under a
  `provenance: asserted` / `source: agent-reported` / `scan_kind: asserted`
  capture **anchored to the stream's latest snapshot for provenance**
  (tsk71/tsk72 — the snapshot says which tree state the value described;
  the `asserted` scan kind keeps it from being read as a scanned set). A
  formula spec is refused, and so is a `count` spec — one asserted fact
  would read as 1 whatever its value. The fact is stamped to match the
  spec's own filter (severity / dim_eq → the `rule` column for
  `oxplow.rule`, dims_json otherwise) and carries ratio components for a
  `ratio` spec (den=100 for `%` so the percent round-trips), so the metric's
  own reads include the asserted number.
- **The desktop reads metrics through SQL** (P4.7, tsk492 — the metric IPC
  reads are gone). `src/metricsSql.ts` builds the queries and row shapes;
  `api.ts` runs them through `query_sql` and returns the rows with the
  query's `reads` (what the views subscribe to): `listMetricSamples` is
  `metric_grid('capture'[, dim])` joined to `v_capture` (one row per capture,
  newest first, bounded to a range in SQL), `listMetricDefinitions` reads
  `v_metric_spec` (v2 adds `entity_json`), `listMetricCatalog` reads
  `v_metric_catalog`. The one metric IPC left is `enable_metrics`, below. The dimension roll-up, per-capture
  findings, measure series/rollup and effort-delta IPC commands were removed
  with the UI that used them (tsk309); agents slice with `metric_grid(…, dim)`
  and read offenders with `metric_findings()`, and per-effort deltas are
  `v_effort_metric_delta`. The agent
  gets the same numbers as prompt text via oxplow-bundled's `metric-deltas`
  advisory (over the stored `v_effort_metric_delta`).
- **Event**: `OxplowEvent::MetricSamplesChanged { stream_id, measures }`, from
  the change loop (the renderer refetches).

## UI

The metric UI is **three pages**, each with one job, all reading the one
fact table — no per-metric UI code. Recorded Metrics is a declared page (the `metrics-recorded` page, oxplow-foundation's
`pages:`, a built-in until the kit can draw it); Metric Detail is the `oxplow/metric` entity page and cross-links to the others in its header. (History: the
configure surface was split off as a fourth "Metric Settings" page in
tsk282/tsk80, then **folded back in by tsk117** — per-metric configuration now
lives on the Metric Detail page; see "The configure surface" below.) Explorer
and Recorded observe; Detail both observes and **writes** (its Configure block).
Authoring a *new* metric is **agent work** (the `oxplow.metric.scaffold` command +
the `/oxplow:new-metric` skill, tsk122) — no page writes one; Recorded Metrics
just carries a Help blurb pointing there.

- **Metrics Explorer** (`MetricsExplorerPage.tsx` wrapping `MetricsExplorer.tsx`)
  — the marquee page and the rail "Metrics" entry (`metricsExplorerRef()`). Header link: "Recorded metrics →" (the "Configure
  metrics →" link died with the Settings page, tsk117). A measure's title
  navigates to the metric's **detail page** (via `onOpenDetail` → `metricRef`).
> ### ⚠️ A seeded spec does NOT mean an enabled metric (tsk87)
>
> The generation seeds a foundation **gauge's** spec only while its collector
> runs: the metric is on by default, or `use:`d when it says `default: off`.
> An off metric has no spec, so `v_metric_spec` holds only metrics with a
> source, and a read or write of an off metric by key says so and how to turn
> it on (`metric_engine::missing_metric` reads the `metric_catalog` table: a
> key the catalog lists but that has no spec is "off, add `- use: <key>`";
> any other key is "no metric"). Producer specs (`builtin_producer_specs`)
> are still seeded unless an `enabled: false` marker prunes them. The page's
> row set stays the **catalog** — it lists what can be turned on, too — with
> the spec joined in by key for presentation metadata.
>
> (The "spec table = the enabled set" phrasing under the collection gate above is
> about the **producer** measures, where disabling does prune. Don't generalize it
> to the foundation's code metrics.)
>
> Note enabling a C# idiom metric here still wouldn't show `0`: `oxplow.ast_hit` is
> `capture_scope: per-path`, whose zero-fill is deliberately suppressed, so a scan
> that matches no files yields no point at all. Nothing auto-detects a project's
> languages.

- **Metrics** (`MetricsPage.tsx`, `PageKind` `"metrics-recorded"` /
  `metricsIndexRef()`) — every **catalogued** metric as a `title · trend
  sparkline · latest value` row (row set = `list_metric_catalog`, the only
  source that knows `use:`; the seeded spec joins in by key for unit /
  direction / thresholds and is null only for an explicitly-disabled metric
  whose spec was pruned). The rail has **Search**, **Show** (`Enabled`,
  default, / `All` — see the box above for why that distinction isn't free;
  pure filtering in `metricsRows.ts`), **Range** and **Branch**. The value
  sits after the sparkline because it *is* that sparkline's last point —
  both read the same range+branch-filtered samples — colored by
  `metricStatusColor` (the shared `metricStatus` classifier:
  target/`fail_at`/direction). Each `<tr>` adopts browser-style click via
  `useRouteDispatch(metricRef(key))`, passing the **sibling chain**
  (`metricSiblings`) so the detail page gets up/down nav (tsk119). A Help
  blurb (`recorded-new-metric-help`) points at the agent for new metrics
  (`oxplow.metric.scaffold` + `/oxplow:new-metric`). Re-runs when a model or measure
  its reads read changed (`useRerunOnChange`, single-flight). **Simplified (tsk309):** the Line value
  stat picker, the Off target mode and the saved-view presets are gone.

> ### Sectioning — one rule, both pages (`buildMetricSections`, tsk81)
>
> Recorded Metrics renders those sections through the shared
> `CollapsibleSections` provider and the kit `Section` with `collapsible` — a chevron on
> each section header, with the **Expand all / Collapse all pair living in the
> details rail** beside the filters (`SectionCollapseControls`, tsk86). Collapsed
> state is kept in the tab entry. The provider wraps the whole
> `<Page>` so its context reaches the rail as well as the body. See
> `.context/usability.md` → "Collapsible page sections".
>
> The section list is built by the pure
> `buildMetricSections(rows, getCategory, getLanguage, getLabel)` in
> `metricCategories.ts`: sections sorted **alphabetically by label**, and rows
> sorted alphabetically within each section (tsk116/tsk118 —
> no hand-kept `CATEGORY_ORDER`, so plugged-in languages/categories slot in
> automatically), **except `static-quality`**, which gets no section of its
> own — its real top-level division is by language, so each language is
> promoted to a top-level section (a peer of Tests / Coverage / Operational)
> and the language-agnostic analysers (`oxplow.analysis.*`) fall under
> **"General"**. (The helper had two callers until tsk117 retired the Metric
> Settings page; it stays the single sectioning rule so any future second
> caller can't drift.)
>
> **Grouping keys off `MetricCatalogEntry.language`** (tsk87). For a foundation
> code metric the catalog takes that slug from the metric's `language:` in
> `extension.yaml`, and the spec carries the same value, so catalog and spec
> agree by construction. `MetricSpec.language`
> is still real read surface: `v_metric_spec.language` is how a query
> filters by language, which silently matches nothing when the column is
> null. Keep it populated.
>
> Note the key segment is **not** the slug: `oxplow.ts.*` is language
> `typescript`. A foundation metric with no `language:` (the language-agnostic code metrics) maps
> to spec `None` — `""` is not a language, and `groupByLanguage` reads null/`""`
> as its "General" bucket.
- **Metric Detail** (`MetricDetailPage.tsx` + the pieces in `MetricDetail.tsx`
  and pure `metricDetailData.ts`, `PageKind` `"metric"`, routed by
  `metricRef(key)`). The metric name is the H1 and its `description` the
  intro. The main column is the trend chart (drag to select a range; charted
  the way the metric rolls up — `defaultChartMode`: sum → cumulative) and the
  paginated recordings (`RecordingsTable`, 25/page; the page is history state).
  Range, branch and slice are the ref's view (`metric:<key>?range=…`, a
  brushed window is `range=<from>..<to>`), drawn by `MetricFilters`. The
  details rail holds those filters, the agg-aware in-range stat and the full
  definition metadata (`MetricStatsRail`), **Add to dashboard ▾** and the
  **Enabled** checkbox. **Simplified (tsk309):** chart mode/scale controls,
  the breakdown card and group filter, the per-kind drill-ins (findings
  table, test tree, coverage lines, top subjects), the Metric Recording page,
  the target-override input and the "In this effort" callout are gone.
  Targets live in config; breakdowns and findings are for agents
  (`metric_grid(…, dim)`, `v_fact` through `oxplow_query_sql`) and lenses.
  Old `metric-recording:<capture>:<key>` tab ids reopen the metric.

> **Definition descriptions (tsk309).** Every metric carries a one-line
> `description` (on `metric_definition`). It's inherent to the definition — set
> once and not overridable by a `use:` entry (`resolve_one` reads `def.description`,
> like trigger). Sources: the foundation's code metrics (the metric's `description:`),
> the always-on producers (`ProducerMetric.description` in `producer_metrics.rs`),
> and config `key:` entries (`MetricEntry.description` → `ResolvedSpec` →
> `spec_definition()`).

### The configure surface (tsk282 → folded into Detail/Recorded by tsk117)

There is **no Metric Settings page anymore**. Configuration was a dedicated
page ("Metrics Catalog" tsk282, retitled "Metric Settings" tsk80) until tsk117
folded it into the surfaces where you already look at a metric:

- **Per-metric config lives on the Metric Detail page** — a "Configure" block
  at the bottom of the details rail: an **Enabled** checkbox
  (`metric-detail-enabled` → `set_metric_enabled`); targets are set in
  `.oxplow/project.yaml`. Failures surface via `recordOpError`.
  The block reads the **catalog entry** (`list_metric_catalog`), NOT the spec:
  a disabled metric's spec is pruned, so the spec-driven page body would
  otherwise dead-end. A disabled metric's detail page renders an enable-prompt
  body (`metric-detail-disabled`) with the Configure block still in the rail —
  that rail is exactly how the metric gets turned back on. The page refreshes
  on `configChanged` as well as `metricSamplesChanged`.
- **"+ New metric" scaffolding is agent-driven (tsk122).** The inline
  `NewMetricBar.tsx` form was **removed** — authoring a metric always needs the
  collector script edited anyway, which is agent work. The scaffold backend
  is now the `oxplow.metric.scaffold` command (see below), and Recorded Metrics' details rail carries
  a Help blurb (`recorded-new-metric-help`) telling the user to ask their agent
  (the `/oxplow:new-metric` skill).
- **Retired with the page:** the per-section **tri-state bulk enable/disable**
  (`GroupCheckbox`/`sectionCheckboxState`, tsk32) — enable/disable is
  per-metric only now (the `set_metrics_enabled` batch IPC is gone;
  `MetricsService::set_metrics_enabled` remains for tests); and the Explorer's "Configure metrics →"
  header link. The `metrics-catalog` page kind is gone from
  `tabState`/`pageRefs`/`pageKinds`/`RailHud`/`App` — a persisted
  `metrics-catalog` tab id no longer matches any render branch, so stale tabs
  **drop silently on restore** (the tab-build chain only pushes matched kinds).

The mechanics behind those controls (unchanged by tsk117):

- **The catalog is a registry of everything available**, NOT a list of metrics
  with recorded data — every metric the system can produce is listed via
  `list_metric_catalog`, even before any sample exists. `catalog()` unions
  **four** sources, deduped by key: (1) the code metrics of oxplow-foundation
  and any other extension (toggleable); (2) project/global `metrics:` entries
  (toggleable); (3) the built-in always-on producers
  (`builtin_producer_metrics()` — tokens, tests, coverage, analysis, effort
  lifecycle, nudges — listed regardless of recorded data so the user sees they
  exist, tsk286); (4) every other seeded `metric_definition` — extension
  metrics and legacy rows. **Every entry is `toggleable: true` (tsk31)** — the
  "always on" class is retired: producers/extensions can be enabled/disabled just
  like code metrics. `catalog()` reads each row's `enabled` from config
  (`config_state`): a metric that says `default: off` is on only when a non-disabled `use:`
  resolves it; the four default-on foundation metrics (`oxplow.todos`,
  `fn_count`, `long_functions`, `high_complexity_fns`: what fills
  `v_function`, so a new project can answer "which functions are longest")
  are used unless the project mentions them. The catalog row's `trigger` for a
  metric that names a collector is that collector's trigger label
  (`trigger_label(&Trigger)`);
  producers/extensions are default-ON unless an `enabled: false` marker
  disables them, and so are those four.
- **Function facts are keyed by symbol ref.** `code_metrics(text, language,
  path)` returns each function with its `id`, `uri` (`oxplow:oxplow/symbol?path=<path>&symbol=<id>`,
  built by the host through `uri::build::symbol_uri`), `container` and
  `kind`; `fn_count`, `high_complexity_fns` and `long_functions` use `uri`
  as the fact's `subject`, so a function has the one identity `v_symbol`
  has. `v_function` is one row per ref and stream (overloads and
  same-named functions are separate rows): `symbol` is the whole ref, and
  `name`, `container` and `kind` are joined from `v_symbol` on
  `(stream_id, uri)` — a function the symbol index doesn't hold isn't listed.
  Token-usage facts use `ai_model:<name>` subjects (`ai_model_ref`).
- **Enable/disable** via the `oxplow.metric.enable { keys, enabled }` command
  (`commands/metric.rs`; the desktop's `enable_metrics` IPC runs it as the
  person) — it computes the new `metrics:` list and hands it to
  `oxplow.config.set`'s core, so it is audited, logged as `config.changed` and
  undoable. Its config shape is
  default-aware (`apply_metric_enabled` + `is_default_on`): a default-OFF
  metric (a `default: off` extension metric / global def) toggles by the presence of a bare
  `use:` entry, while a default-ON metric (producer/extension) toggles by an `enabled: false`
  **marker** in `.oxplow/project.yaml` (which holds only `use:` entries and
  these markers). the generation then **reconciles** the
  `metric_spec` table down to exactly the enabled set — upsert the enabled,
  `delete_spec` the disabled — so all spec-driven reads
  (Explorer/Recorded/Detail/effort-deltas/MCP) go empty for a disabled metric,
  and its producer's collection stops via the `measure_has_active_spec` gate
  (see the producer section: **base data is not collected when no active
  metric consumes its measure** — shared-measure families like
  `oxplow.tokens`/`oxplow.test_case` keep flowing until *all* their metrics
  are off). Historical facts are never deleted, so re-enabling restores the
  chart.
- **Targets** are config: a `use:` entry's `target` / `warn_at` / `fail_at`
  in `.oxplow/project.yaml` (the project's choices; never definitions). **Trigger is inherent to the definition** —
  `resolve_one` reads it from the definition (like `compute`) and a `use:`
  entry can't override it (tsk290).
- **`oxplow.metric.scaffold` (a command since P4.8; a template since tsk391; now the
  metric extension)** returns the files of an extension (see
  "Authoring surface"): `MetricsService::metric_scaffold` → `MetricScaffold {
  name, dir, files: [{ path, content }] }`, the same
  `metric_scaffold::extension_files` the SDK's `Kind::Metric` writes, and
  **writes nothing**. The agent writes the files with its own tools (the
  write guard and its worktree apply) and the catalog reseeds when the
  extension loads. It no longer edits `.oxplow/project.yaml` and has no
  `gauge` wording or default. Global metrics are authored by hand in the
  global config dir (there are no global collectors). The runner reads a
  collector's `entry` from its owner's extension folder (`collector_script_text`; oxplow-foundation's scripts are embedded like any bundled extension file).

Metrics are also surfaced **organically off the Metrics pages** (tsk250): the
effort review (`DiffViewPage`'s `effort.review.details` slot) shows the oxplow-bundled
`effort-metric-deltas` lens — the metrics the effort moved, before→after with
Δ, better/worse and any threshold crossing, over `v_effort_metric_delta`
(tests, coverage, analysis, tokens and nudges have their own lenses and are
left out). A row links to the metric's detail page.

Catalog reads/writes: `v_metric_catalog` (the `metric_catalog` table, V106,
which the generation rewrites from `MetricsService::catalog()` on every
reseed) and the `oxplow.metric.enable` command — consumed by the Metric Detail
Configure block and the Metrics rows.
**Scaffolding is not here** (tsk122): its UI button was retired, and it is
now the `oxplow.metric.scaffold` command (P4.8), which calls
`MetricsService::metric_scaffold`.
Token and page analytics are oxplow-bundled lenses (`usage`) over
`v_token_usage` / `v_page_visit`; `page_visit`/`usage_event` are deliberately
**not** projected into the metric substrate — see the producers note above.

## Adding a metric (today)

1. Pick a namespaced `key` and a kind.
2. In the relevant producer, `upsert_definition` it and `record_sample` with the
   value (+ components for ratios), subject, and dims.
3. It then appears in MCP/IPC reads and on the Metrics page automatically — no
   UI code per metric.

## Authoring surface (the four config blocks — epic tsk12, E)

**A code metric is an extension (current).** `oxplow extension new metric
<name> [--language rust] [--glob '**/*.rs']` (or the `oxplow.metric.scaffold`
command, which returns the same files for the agent to write — one template,
`metric_scaffold::extension_files`) makes an extension source under
`.oxplow/extensions/<name>/` with all four blocks, namespaced `<ns>.*` (the
name with `_` for `-`):

- `measures:` one per-path (`captureScope: per-path`), semi-additive measure on
  files, `<ns>.complex_functions` — per-path because a snapshot's collector
  rescans only the files it changed, semi-additive because the latest
  count is the answer;
- `collectors:` `<ns>.scan`, Starlark, `trigger: { on: [snapshot.taken] }`,
  over `source_files()` + `code_metrics()` (the starter counts each
  file's functions over `LIMIT`, records one fact per file with the worst
  function's `line` and `dims: { <ns>.area: <top folder> }`; a clean file
  records nothing, and rescanning it supersedes its old fact);
- `dimensions:` `<ns>.area`, which the facts' `dims` use and the metric slices by;
- `metrics:` `<ns>.complex_functions`, `sum`, `lower-better`,
  `displayKind: findings` (its facts are located), `sliceableDims: [<ns>.area]`;
- a Code launcher lens over `metric_findings('<ns>.complex_functions')`
  whose `path` column links `file` at the `line` column, and an intent
  example + `fixtures/basic.yaml` in the fact-collector form `extension
  test` runs (`input: { collector, files: {path: text} }`, `expect: {
  facts: [...] }`, each fact compared on the keys the expectation names).

It checks with no warnings and tests clean as written
(`crates/oxplow-sdk/tests/just_works.rs`), and the whole loop — capture,
carry-forward across a delta, slicing by area, the lens rows, the refusals
and an oversized file — is held by `metrics_authoring_tests.rs` in
`oxplow-app`. Edit the collector, the fixture and the lens; the
project-config path below is for a metric that is the project's own
(e.g. a report-derived or `exec` collector).

An extension (the project's own, or the user-global library for metrics) declares
metrics in YAML — no Rust per metric. The project's `.oxplow/project.yaml` only
turns them on (`use:`, with optional `target` / `warnAt` / `failAt`) or off
(`enabled: false`); its `collectors:`, `measures:`, `dimensions:` and `key:`
metrics are refused with an error naming
`.oxplow/extensions/project/extension.yaml`. The substrate is dimensional, so authoring splits into **four orthogonal
blocks** (matching the real cardinality): `measures:` (fact TYPEs) ← `collectors:`
(fact PRODUCERs — a collector with `facts:`) → facts → `metrics:` (read SPECs),
sliced by `dimensions:`. Metrics/measures/dimensions are parsed, validated and
resolved in `crates/oxplow-config/src/lib.rs` (`MetricEntry`→`ResolvedSpec` +
`resolve_metrics`; `load_global_metric_entries`); collectors in
`crates/oxplow-config/src/collectors.rs` (`CollectorSpec`, the same parser for
`extension.yaml` — see
[semantic-layer.md](./semantic-layer.md) "Collectors"). The fact engine
(`MetricsService`) seeds a `metric_spec` per resolved metric and runs each
**fact collector** on its `trigger`. (Until P7.B3 the producer block was
`gauges:`; it is now an unknown key, tsk865.)

```yaml
# .oxplow/extensions/project/extension.yaml
measures:                             # the fact TYPE the collector records
  - key: repo.todo_count
    subjectKind: file
    unit: count
    temporalSemantics: semi-additive  # additivity OVER TIME
collectors:                           # the PRODUCER (runs a script, records facts)
  - id: repo.todo
    doc: TODO comment scan
    runtime: starlark                 # starlark | jaq | exec (own extension only, needs permissions.process)
    entry: collectors/todo.star       # relative to the extension folder
    trigger: { on: [snapshot.taken] } # manual | { every: 15m } | { on: [<types>], where?: {...}, settle?, at_most?, idle?, force? }
    facts: [repo.todo_count]          # declare-to-collect allow-list
    # report: { path: target/x.json, format: json }   # text|json|xml|lcov|lines
    # input: "SELECT … :effort_id"    # starlark/jaq: rows as input.rows
metrics:                              # the read SPEC (the chartable metric)
  - key: repo.todo_count              # DEFINE — a measure aggregation
    sourceMeasure: repo.todo_count
    aggregation: sum                  # count|sum|avg|min|max|last|ratio (within a capture)
    direction: lower-better
    unit: count
    displayKind: gauge                # gauge|findings|test|coverage|event
    filter: { minValue: 1 }           # optional predicate before aggregating
    sliceableDims: [oxplow.language]
  - use: myglobal.todo_density        # ENABLE a catalog metric (+ threshold overrides)
    target: 5
```

- A **metric** no longer computes anything — it's a pure spec (`sourceMeasure` +
  `aggregation` + optional `filter`, OR a `formula: {op,left,right}` over other
  metrics). Two-axis aggregation: `aggregation` combines facts *within a capture*;
  the source measure's `temporalSemantics` governs the cross-time collapse. A
  `use:` may only re-target thresholds; the structural fields are inherent.
- The **fact collector**'s script gets `input = {report?, rows?, event?}` (the
  parsed `report:`, the `input:` query's rows, the trigger event as the
  event envelope — extensions.md "The event envelope") and the
  snapshot tree it runs over through `files(glob)` / `source_files()` /
  `ast_query(text, language, sexpr)` / `duplicate_blocks(min_lines)` (see
  [collection.md](./collection.md)) — `tree.read`, which a fact collector
  implies. It may also read the semantic layer (`scope("sql.read", …)`)
  or ask a model (`ai_*`) when it declares `needs: [sql.read]` /
  `[ai.compute]` — the latter only when its extension declares
  `permissions.ai`; `MetricsService::given` builds them
  (extensions.md "Services"). It returns `{ "facts": [
  {measure, value, subject?, path?, line?, rule?, num?, den?, dims?} ] }` —
  one atomic fact per subject (never a baked total). `facts_of` refuses any
  other output (the old `samples` / `findings` shape is gone); a fact on a
  measure not in the collector's `facts:` or not defined is dropped.
- **Two metric scopes** (`global`, `extension:<name>`), precedence global <
  each extension by key, with the project's `use:` layering thresholds
  (collectors' owner is always an extension name):
  - **oxplow's code metrics** live in the required bundled extension
    **oxplow-foundation** (`extensions/oxplow-foundation/extension.yaml`,
    `collectors:` and `metrics:`; scripts under
    `extensions/oxplow-foundation/collectors/<lang>/*.star`, `<lang>` being
    rust, ts, clojure, csharp, or `code` for the language-agnostic ones,
    embedded through `bundled_extensions.rs` like any bundled extension file).
    Nothing is built in: oxplow's own are this extension's. Their scope is `extension:oxplow-foundation`, their collector refs
    `collector:oxplow-foundation/<id>`. Each is authored through the **public**
    surface (`files()`/`ast_query()`) — no privileged Rust path — and verified by
    golden tests over a fixture corpus (`foundation_collectors_tests.rs`, which
    read each script from the bundled foundation by collector id). Each metric
    names its collector (`collector: <id>`) and the collector declares its
    `facts:`. Four are on by default (`oxplow.todos`, `fn_count`,
    `high_complexity_fns`, `long_functions`); the other 12 say `default: off`
    and a project turns one on with `metrics: - use: oxplow.<lang>.<name>`. A
    collector that metrics name runs only while one of them is on
    (`MetricsService::fact_collectors`). A collector's trigger is its own
    (`on` + `where` in `extension.yaml`; the Catalog shows it as
    `on snapshot.taken`). Three families:
    - **Language-agnostic code metrics** (tsk314) — one metric, all languages —
      `oxplow.todos`, `oxplow.fn_count`, `oxplow.high_complexity_fns`,
      `oxplow.long_functions`, plus **`oxplow.doc_coverage`** (tsk125 — a per-file
      RATIO of documented-public ÷ public over `code_metrics()`'s `has_doc`;
      measure `oxplow.doc_coverage`, V69, per-path; spec is a `ratio` %,
      higher-better, not a count). Declared with no
      `language:`; the scripts (under the foundation's `collectors/code/`) sweep the
      `source_files()` reader and call a capability (`code_metrics()` /
      `markers()`), so the per-language knowledge lives in `oxplow-code-metrics`,
      not the metric. See "Language-agnostic capability layer" below.
      `source_files()` **excludes codegen output** (tsk68) via
      `oxplow_code_metrics::is_generated_source` — a `generated` path segment,
      a `.generated.`/`_generated.` basename infix, or a do-not-edit-style
      header (`@generated`, `do not edit`, `autogenerated`) in the first 10
      lines. Otherwise a 3k-line tauri-specta bindings file reads as one giant
      "function" and dominates every fn_length/complexity tail metric. The
      `files(glob)` reader does NOT filter — collectors choose their own
      corpus.
    - **Language-idiom metrics** (`oxplow.<lang>.*`) — concepts specific to one
      language: **Rust** (`unsafe_blocks`, `unwrap_expect_calls`,
      `panic_macros`), **TypeScript** (`any_usage`, `non_null_assertions`,
      `console_calls`, `ts_ignore`), **Clojure** (`defn_count`), **C#**
      (`empty_catch`, `blocking_async_calls`).
    - **Whole-tree scans** (`TREE`) — **`oxplow.duplicate_lines`** (P7.B5,
      tsk388): `duplicate_blocks(min_lines)` (the host builtin over
      `oxplow_code_dup::detect_duplicates`) and one fact per side of each
      block (value = its line count, subject `block:<path>:<start>-<end>`);
      the spec is a `sum`. It runs on `snapshot.taken` with
      `where: { trigger: git_refs }` — every ref move logs one, an unchanged
      tree included — paced (the collector's trigger: `settle: 60s`,
      `at_most: 15m`), so a burst of moves is one scan of the latest tree; and
      its collector is `whole_tree: true`: it always reads the
      reconstructed tree (`build_full_file_map`, `scan_kind = full`), and it
      alone runs on a take that recorded no files (a delta collector has no
      delta there). A clean tree records an empty capture, which clears the
      metric (tsk44).

    This repo dogfoods the language-idiom Rust/TS sets + all four unified code
    metrics through `use:` entries in its own `.oxplow/project.yaml`. The
    complexity/`code_metrics()`-backed collectors and the C# grammar
    (`tree-sitter-c-sharp` → `Language::CSharp` in `oxplow-code-metrics`) landed in
    tsk229/tsk230.
  - **user-global** — `global_config_dir()/{metrics,measures,dimensions}/*.yaml`,
    shared across projects, hot-reloaded by the config watcher. Global
    measures are active everywhere automatically; a global *metric* is enabled
    per-project with a `use:`. There are no global collectors.
  - **extension** — an enabled extension's `extension.yaml` `measures:` /
    `metrics:` / `collectors:` (its fact collectors sandboxed: starlark or jaq;
    `exec` only for the project's own extension,
    `.oxplow/extensions/project/` by convention, with collector scripts under
    its `collectors/`).

  `use:` references a catalog metric key and layers threshold overrides; `key:`
  defines a new spec. `oxplow.*` keys belong to the extension whose namespace
  is `oxplow` (`oxplow_config::reserved_key`; the project may `use:` one but no
  other extension may define under it). Collectors are definition-only (declared,
  never `use:`d).
- Validation mirrors the extension rules: namespaced keys, extension-relative
  `entry` (no `..`), known runtime/aggregation/displayKind/trigger/direction;
  `report` only on a fact collector; `entities` or `facts`, not both;
  a `key:` metric must set exactly one of `sourceMeasure`/`formula`; a `use:` with
  an unknown key resolves to a warning (skipped), not an error.

The in-oxplow agent authors these on request via the **`oxplow-metrics`** skill
+ the **`/oxplow:new-metric`** command (assets in `crates/oxplow-agent-text/`,
materialized for Claude/Codex/opencode) — "make a metric that counts TODOs" →
the measure+collector+metric trio + script + verification (`oxplow.collector.sync`
runs it now), no oxplow-team involvement.
The skill's fast path is the **`oxplow.metric.scaffold` command** (P4.8) →
`MetricsService::metric_scaffold`, which returns the metric extension's files
(measure, collector, dimension, metric, lens, fixture; "Authoring surface")
as a template the agent writes and adapts; `oxplow extension check` and
`extension test` verify it before `oxplow.collector.sync` runs it. A metric
that is the project's own (a report-derived or `exec` collector) is
hand-authored in the four blocks of `.oxplow/extensions/project/extension.yaml`.

## Targets & feedback (advise-only, P5/tsk220)

A definition's `target` / `warn_at` / `fail_at` (interpreted via `direction`) are
the single source of red/green — no hardcoded UI ramps. The coverage cut-offs
are the foundation's `oxplow.coverage.abs_pct` spec (`target: 80`, `warnAt:
80`, `failAt: 50`), which a project tunes with `metrics: [{ use: … }]`;
oxplow-bundled's effort verdict judges diff coverage against its `warnAt`.

**One rule for crossing.** A value *past* a threshold has crossed it; a value
*at* it has not (`target: 80, warnAt: 80` means 80 is fine). The server's
`metric_engine::threshold_state` (findings' severity, `v_effort_metric_delta.crossing`,
the effort panel) and the Metrics page's `metricsRows.ts::metricStatus` (rows
and dashboard tiles, three tiers) draw that same line. See
[theming.md](./theming.md).

Feedback is **advisory — oxplow never blocks**. It lives in the bundled
`oxplow-bundled` extension as **advisories** (see
[extensions.md](./extensions.md) → "Advisories"), SQL over the stored
per-effort views, not in core:

- **`metric-deltas`** (prompt, every turn): "# Metric deltas (this effort)",
  one `title: baseline → current (Δ ±N)` line per moved **code** metric
  (`v_effort_metric_delta`; operational `agent.*`/`effort.*`/`task.*` and
  `event` kinds skipped), then "(Advisory — for awareness, not gating.)".
  The numbers are the same `effort_metric_deltas` roll-up the task page
  shows (file-attributed for snapshot-scan metrics).
- **`threshold-crossed`** (prompt, once per metric per effort): "⚠ <title>
  crossed its warn/fail threshold (N)", from the delta's `crossing`
  (`threshold_state`). This used to be a marker on the delta line; it's now
  its own block.

All three reach the agent as nudges (`v_agent_nudge`), taken by the next
prompt or tool call (`.context/extensions.md` "Advisories").
Disabling `oxplow-bundled` turns them off.

## Performance: the `producers_for_measure` memo (tsk130)

`SqliteFactStore::producers_for_measure` was the single biggest backend CPU sink
in the tsk129 profile — **309 s inclusive, ~46% of all backend CPU**. Not a
missing index (`idx_fact_measure_capture` covers it): a **call-volume** problem.
It runs once per measure in `metric_cube::build_measure`, the cube read fold, and
`metric_engine::partial_state_facts`, so every metric read recomputed an answer
that only changes when new facts land.

It is now **memoized by `measure_id`**, and two things about that are load-bearing:

- **The memo lives on `Database` (`QueryMemo`), not on the store.** The app builds
  *several* `SqliteFactStore` instances over one `db.clone()` — `Services` holds
  one, `MetricEngine` constructs another, tests make more. A per-store cache
  would let a write through one instance leave another's copy stale, so a new
  producer's facts would be silently missing from reads until the next write.
  `Database` is `Clone` over an `Arc`, so the memo is shared by exactly the
  instances that share the data. `fact_store.rs` has a test that writes through a
  *second* store and asserts the first sees it — a per-store cache fails it.
- **Invalidation is per measure and generation-guarded.** A fact write
  forgets only the measures it wrote: each measure has its own generation,
  bumped after the commit by `record_facts` / `record_test_run` and by
  `facts_committed(measures)` (callers that wrote through `record_facts_tx`
  in their own transaction). A global clear made token facts, landing every
  ~18 s, keep every other measure's memo cold. The read side records the
  measure's generation it queried under and **declines to cache** if it
  changed, because a query that started before a write and finished after
  it would otherwise install a result missing the new producer, with nothing
  to clear it until the next write.

The same per-measure entry memoizes the measure's slice reads,
`distinct_slice_keys` and `representative_facts_by_slice` (each a GROUP BY
over the measure's whole history, which the effort deltas ran per spec,
per effort, per recompute). A **deletion** of facts (a capture prune; facts
cascade) can't cheaply say which measures it touched, so it bumps the
memo's epoch (`invalidate_all`) and every entry is forgotten.

The cube builder skips a measure whose producers recorded no capture
since its last complete pass and whose cube epoch hasn't moved
(`capture_token_for_producers` + `cube_epoch`, remembered in memory): a
pass on every write burst used to load all of every measure's captures to
find nothing past the watermark. Every cube wipe bumps the epoch, so a
wipe always rebuilds.

If you add another path that inserts into `fact`, it must call
`db.memo().invalidate_measures(<the measures written>)` after committing;
one that deletes facts, `db.memo().invalidate_all()`.

## Gotchas

- **Config write-back is generic** (tsk355). Every settings write rewrites
  `metrics:` in `project.yaml` (`use:` entries and `enabled: false` markers) through
  `oxplow_config::minimal_yaml`: the entry's own serde form, with nulls and empty
  lists and maps dropped. A new field on an entry struct is written back
  automatically. The per-field writers this replaced dropped entity metrics'
  `entity` / `where` / …, so the next load failed validation.
- **Provenance is the spine** (carried from collection.md): in-process/parsed →
  `observed`; agent-asserted / exec-tier → `asserted` / `exec:<name>`. The
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

## Slice By on the metric page (P6.F1)

`MetricDetailPage`'s Details rail has a **Slice By** select over
`v_dimension`. Choosing one re-reads the captures with
`listMetricSamples(key, limit, groupBy)` (`metricSeriesSql`'s grouped
`metric_grid('capture', '<dimension>')`), and the page draws one chart per
dimension value (`seriesByGroup`, the same small multiples as a lens line
chart).

The **Metrics list** has the same select in its Filters rail ("Slice by",
`MetricsPage`). Sliced, every metric keeps its row (its overall sparkline and
latest value) and lists its **top three groups beneath it** — each group's
latest value and trend, ranked by latest value (`sliceGroups`,
`metricsRows.ts`), the rest counted as "+ N more". A metric with nothing on
that dimension says "Not sliced by <dimension>". The groups follow the page's
range and branch filters; a group is a value of the dimension, not a metric,
so it opens no page (the detail page, sliced the same way, is where one
group's chart lives). It costs a second read per metric while a slice is on.


## Coherent extension metric publication

Production boot prepares metric declarations from the same immutable extension
snapshot and captured project choices as models, commands and vocabulary.
`metrics_service/generation.rs` resolves measures, dimensions, builtin/configured
specs and the available catalog without writing. Candidates allow at most 4096
entries per category and 16 MiB serialized metadata. Inside the publication
transaction, entity SQL is checked against candidate views, measures/dimensions/
specs are upserted, removed specs are reconciled and `metric_catalog` is replaced.
Any diagnostic or storage failure aborts the candidate; published readers keep
the prior metric rows and declarations. Measures owned by disabled extensions
remain with their history. The required-bundled repair baseline does not prune
project measures or cascade away their facts.

Captured metric choices and global definitions publish under the generation
writer along with the catalog. Fact-collector resolution reads these frozen
choices; catalogue reads pin before joining declarations and stored specs.
Production metric reseed requests the activation owner instead of writing a
second catalog; the independent raw-catalog metric reactor is retired. State
value recapture happens after publication in a reader scope, outside the
publication transaction. Until a generation is activated, metrics seed directly.
No script, process, network call or capture executes inside catalog publication.
Transaction-composable fact-store upserts preserve change-detected cube
invalidation for capture-scope and promoted-dimension changes. Runtime
acceptance checks and migration execution remain outstanding.

Paced collector selection and execution retain the same generation reader;
advisory selection/evaluation also pins before combining runtime declarations
with queries.
