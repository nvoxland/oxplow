import { useEffect, useMemo, useState } from "react";
import { formatMetricValue, formatMetricValueExact } from "../components/format";

import {
  type MetricSpec,
  type SeriesPoint,
  listMetricCatalog,
  listMetricDefinitions,
  listMetricSamples,
  subscribeOxplowEvents,
} from "../api.js";
import {
  CollapsibleSection,
  CollapsibleSections,
  SectionCollapseControls,
} from "../components/CollapsibleSections.js";
import { metricRef } from "../tabs/pageRefs.js";
import { Page } from "../tabs/Page.js";
import { useRouteDispatch } from "../tabs/RouteLink.js";
import type { NavSiblings } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import { buildMetricSections } from "./metricCategories.js";
import {
  DEFAULT_RANGE_KEY,
  RANGE_PRESETS,
  branchOptions,
  filterByBranch,
  filterByRange,
  rangeFromPreset,
} from "./metricDetailData.js";
import {
  DEFAULT_SHOW_MODE,
  SHOW_MODES,
  type ShowMode,
  filterMetricRows,
  isOffTarget,
  metricStatus,
  metricSiblings,
} from "./recordedMetricsRows.js";
import {
  DEFAULT_LINE_STAT,
  LINE_STATS,
  type LineStat,
  lineStatValue,
} from "./recordedMetricsStat.js";

/** One listed metric. Identity/enabled/grouping come from the **catalog** (the
 *  only source that knows about `use:`); `def` is the seeded spec, which carries
 *  the presentation metadata (unit, direction, thresholds) and is null when the
 *  metric was explicitly disabled and its spec pruned. */
type Row = {
  key: string;
  title: string;
  category: string | null;
  language: string | null;
  enabled: boolean;
  def: MetricSpec | null;
  latest: SeriesPoint | null;
  samples: SeriesPoint[];
};

const SAMPLE_LIMIT = 200;



/** Inline-SVG sparkline of a metric's values over time (oldest → newest). */
function Sparkline({ values, color }: { values: number[]; color?: string }) {
  if (values.length < 2) return <span style={{ opacity: 0.35 }}>—</span>;
  const w = 90;
  const h = 22;
  const pad = 2;
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const pts = values
    .map((v, i) => {
      const x = pad + (i / (values.length - 1)) * (w - 2 * pad);
      const y = h - pad - ((v - min) / range) * (h - 2 * pad);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg width={w} height={h} style={{ display: "block" }} aria-hidden>
      <polyline points={pts} fill="none" stroke={color ?? "var(--accent, #58a6ff)"} strokeWidth={1.5} />
    </svg>
  );
}

/** Color a value against the metric's `target`/`fail_at` + `direction` — the
 *  data-driven successor to the hardcoded coverage 50/80 ramp (tsk220).
 *  Delegates to the shared `metricStatus` classifier so the row color and the
 *  Off-target filter can't disagree (tsk121). */
function statusColor(def: MetricSpec, value: number): string | undefined {
  switch (metricStatus(def, value)) {
    case "ok":
      return "var(--ok, #3fb950)";
    case "fail":
      return "var(--err, #f85149)";
    case "warn":
      return "var(--warn, #e5a50a)";
    default:
      return undefined;
  }
}

/** The color for a rendered CHANGE: green when the move improved the metric
 *  per its `direction`, red when it worsened it — the same semantics
 *  `EffortMetrics`' delta chips use. Neutral direction / zero / unknown stay
 *  uncolored: silence is the dominant path. */
function changeColor(def: MetricSpec | null, delta: number | null): string | undefined {
  if (!def || delta == null || delta === 0 || def.direction === "neutral") return undefined;
  const improved = def.direction === "higher-better" ? delta > 0 : delta < 0;
  return improved ? "var(--ok, #3fb950)" : "var(--err, #f85149)";
}

/** One metric row: title · trend sparkline · the rail-selected stat. A `<tr>`
 *  that adopts browser-style click via `useRouteDispatch` (plain → detail
 *  in-tab, modifier/middle/right → new tab). */
function RecordedRow({
  row,
  stat,
  onOpenPage,
  siblings,
}: {
  row: Row;
  stat: LineStat;
  onOpenPage?: (ref: TabRef) => void;
  siblings?: NavSiblings;
}) {
  const { def, latest, samples } = row;
  const color = def && latest ? statusColor(def, latest.value) : undefined;
  const { handlers } = useRouteDispatch(metricRef(row.key), { onNavigate: onOpenPage, siblings });
  // The stat is computed over the SAME filtered samples the sparkline plots
  // (tsk82's invariant, generalized by tsk115): the number that terminates the
  // line always describes the line. `change` gets an explicit `+` and the
  // improved/worsened color; `distance` is also signed but keeps the metric's
  // status color (it describes the latest value's standing, like the level
  // stats); the target stats need the def's `target` (tsk120).
  const shown = lineStatValue(samples, stat, def?.target ?? null);
  const valueColor = stat === "change" ? changeColor(def, shown) : color;
  const signed = stat === "change" || stat === "distance";
  // Percent-of-target is a percent regardless of the metric's own unit;
  // distance is in the metric's unit; everything else too.
  const unit = stat === "pctTarget" ? "%" : def?.unit;
  return (
    <tr
      onClick={handlers.onClick}
      onAuxClick={handlers.onAuxClick}
      onContextMenu={handlers.onContextMenu}
      style={{ borderTop: "1px solid var(--border, #2a2a2a)", cursor: "pointer" }}
    >
      <td style={{ padding: "6px 8px", fontWeight: 600 }}>{row.title}</td>
      <td style={{ padding: "6px 8px" }}>
        <Sparkline
          values={samples
            .slice()
            .reverse()
            .map((s) => s.value)}
          color={color}
        />
      </td>
      <td
        style={{ padding: "6px 8px", fontWeight: 600, color: valueColor }}
        title={shown != null ? formatMetricValueExact(shown, unit) : undefined}
      >
        {shown != null
          ? `${signed && shown > 0 ? "+" : ""}${formatMetricValue(shown, unit)}`
          : "—"}
      </td>
    </tr>
  );
}

const sel = { fontSize: 12, width: "100%" } as const;

/**
 * Recorded Metrics — every catalogued metric as a `title · trend sparkline ·
 * latest value` row, organized as **one table per section** under headings, via
 * the shared `buildMetricSections` (Code gauges / Tests / Coverage / then one
 * top-level section **per language** for static analysis / Operational)
 * (tsk81). A right-side panel scopes the latest/trend by a preset time range
 * (default 7 days) and branch, picks Enabled (default) / All, and holds the
 * Expand/Collapse-all controls. Rows open the per-metric detail page
 * (`metricRef`), which is also where a metric is enabled/disabled and its
 * target set (tsk117). Authoring a NEW custom metric is agent work now (the
 * "+ New metric" scaffold form was retired in tsk122 for agent-driven authoring
 * via the `/oxplow:new-metric` skill + the `scaffold_metric` MCP tool); the rail
 * carries a Help blurb pointing there.
 * Live on `metricSamplesChanged` (debounced) and `configChanged`.
 *
 * **The row set is the CATALOG, not the spec table (tsk87).** Only the catalog
 * knows about `use:`: a built-in gauge keeps its seeded spec when merely
 * un-`use:`d (it just never runs), so reading specs alone listed the bundled
 * C#/Clojure idiom gauges in a Rust/TS repo as permanent `—` rows while Metric
 * Settings showed the same rows unchecked. The spec joins in by key for the
 * presentation metadata (unit / direction / thresholds) and is null only for an
 * explicitly disabled metric, whose spec is pruned.
 */
export function RecordedMetricsPage({ onOpenPage }: { onOpenPage?: (ref: TabRef) => void } = {}) {
  const [rows, setRows] = useState<Row[]>([]);
  const [loading, setLoading] = useState(true);
  const [rangeKey, setRangeKey] = useState<string>(DEFAULT_RANGE_KEY);
  const [branch, setBranch] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [showMode, setShowMode] = useState<ShowMode>(DEFAULT_SHOW_MODE);
  const [lineStat, setLineStat] = useState<LineStat>(DEFAULT_LINE_STAT);

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void Promise.all([listMetricCatalog(), listMetricDefinitions()]).then(
        async ([catalog, defs]) => {
          const specs = new Map(defs.map((d) => [d.key, d]));
          const built = await Promise.all(
            catalog.map(async (entry) => {
              const def = specs.get(entry.key) ?? null;
              // No spec ⇒ no spec-driven reads ⇒ don't pay for the IPC.
              const samples = def ? await listMetricSamples(entry.key, SAMPLE_LIMIT) : [];
              return {
                key: entry.key,
                title: entry.title,
                category: entry.category,
                language: entry.language,
                enabled: entry.enabled,
                def,
                latest: samples[0] ?? null,
                samples,
              };
            }),
          );
          if (!cancelled) {
            setRows(built);
            setLoading(false);
          }
        },
      );
    };
    refresh();
    // A refresh is one `listMetricSamples` per catalogued metric, and each of
    // those walks its measure's whole fact history (`oxplow.test_case` alone is
    // ~235k facts) — ~20 CPU-seconds a go. The OTLP token ingest emits
    // metricSamplesChanged on every agent turn, so an un-debounced reload made
    // oxplow's CPU proportional to how hard the agent was working (tsk91). Same
    // bug tsk75 fixed for `EffortMetricsBlock`, same trailing-debounce fix:
    // coalesce a turn's burst of exports into one reload.
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = subscribeOxplowEvents((e) => {
      // configChanged is user-action-rate (an enable toggle on the detail
      // page, a scaffold create, a project.yaml edit) — refresh immediately.
      // Only the OTLP-burst metricSamplesChanged needs the debounce.
      if (e.kind === "configChanged") {
        refresh();
        return;
      }
      if (e.kind !== "metricSamplesChanged") return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(refresh, 2_500);
    });
    return () => {
      cancelled = true;
      off();
      if (timer) clearTimeout(timer);
    };
  }, []);

  const branches = useMemo(() => branchOptions(rows.flatMap((r) => r.samples)), [rows]);
  // Which metrics are LISTED (Show mode + search), each scoped to the range +
  // branch — an in-scope metric with no recording in the window stays listed
  // and just shows "—". We window FIRST so `off-target` can test each row's
  // latest value *within the selected filters* (the same value that colors the
  // row), then narrow: filterMetricRows handles enabled-ness + query, and the
  // off-target value test runs against the windowed latest (tsk121).
  const viewRows = useMemo(() => {
    const range = rangeFromPreset(rangeKey, Date.now());
    const windowed = filterMetricRows(rows, showMode, query).map((r) => {
      const samples = filterByBranch(filterByRange(r.samples, range), branch);
      return { ...r, latest: samples[0] ?? null, samples };
    });
    if (showMode !== "off-target") return windowed;
    return windowed.filter((r) => r.latest != null && isOffTarget(r.def, r.latest.value));
  }, [rows, rangeKey, branch, query, showMode]);

  const sections = useMemo(
    () =>
      buildMetricSections(
        viewRows,
        (r) => r.category,
        (r) => r.language,
        (r) => r.title,
      ),
    [viewRows],
  );

  // The up/down sibling chain a drilled-into detail page steps through
  // (tsk119): the rendered sections flattened in visual order.
  const siblings = useMemo(() => metricSiblings(sections, (key) => metricRef(key)), [sections]);

  return (
    // The provider wraps the whole Page so its context reaches BOTH the details
    // rail (which holds the Expand/Collapse-all controls) and the body (which
    // holds the sections) — `rightRail` is created here but rendered inside
    // Page's subtree, and context follows the render tree.
    <CollapsibleSections pageKey="metrics-recorded" testIdPrefix="recorded">
    <Page
      testId="page-metrics-recorded"
      title="Recorded Metrics"
      layout="details"
      rightRailTitle="Filters"
      rightRail={
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
            <span style={{ opacity: 0.6, fontSize: 12 }}>Search</span>
            <input
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Filter by name…"
              data-testid="recorded-search"
              style={sel}
            />
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
            <span style={{ opacity: 0.6, fontSize: 12 }}>Show</span>
            <select
              value={showMode}
              onChange={(e) => setShowMode(e.target.value as ShowMode)}
              data-testid="recorded-show-mode"
              title="Enabled lists only metrics this project has turned on; All also lists the ones it hasn't; Off target lists just the enabled ones missing their target in the current window."
              style={sel}
            >
              {SHOW_MODES.map((m) => (
                <option key={m.key} value={m.key}>
                  {m.label}
                </option>
              ))}
            </select>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
            <span style={{ opacity: 0.6, fontSize: 12 }}>Range</span>
            <select
              value={rangeKey}
              onChange={(e) => setRangeKey(e.target.value)}
              data-testid="recorded-range"
              style={sel}
            >
              {RANGE_PRESETS.map((p) => (
                <option key={p.key} value={p.key}>
                  {p.label}
                </option>
              ))}
            </select>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
            <span style={{ opacity: 0.6, fontSize: 12 }}>Branch</span>
            <select
              value={branch ?? ""}
              onChange={(e) => setBranch(e.target.value || null)}
              data-testid="recorded-branch-filter"
              style={sel}
            >
              <option value="">All branches</option>
              {branches.map((b) => (
                <option key={b} value={b}>
                  {b}
                </option>
              ))}
            </select>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
            <span style={{ opacity: 0.6, fontSize: 12 }}>Line value</span>
            <select
              value={lineStat}
              onChange={(e) => setLineStat(e.target.value as LineStat)}
              data-testid="recorded-line-stat"
              title="What the number at the end of each line shows — always computed over the plotted range/branch window."
              style={sel}
            >
              {LINE_STATS.map((o) => (
                <option key={o.key} value={o.key}>
                  {o.label}
                </option>
              ))}
            </select>
          </div>
          {/* Not a filter, but this rail is the page's control panel — the
              controls self-hide while there are no sections to act on. */}
          <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
            <SectionCollapseControls />
          </div>
          {/* Authoring a custom metric is agent work now (the "+ New metric"
              scaffold form was retired, tsk122): the agent wires up the trio +
              gauge script correctly via the /oxplow:new-metric skill. This blurb
              points the user there. */}
          <div
            data-testid="recorded-new-metric-help"
            style={{
              display: "flex",
              flexDirection: "column",
              gap: 4,
              marginTop: 4,
              paddingTop: 10,
              borderTop: "1px solid var(--border, #2a2a2a)",
            }}
          >
            <span style={{ opacity: 0.6, fontSize: 12 }}>New metric</span>
            <span style={{ fontSize: 12, lineHeight: 1.5, opacity: 0.85 }}>
              Ask your agent to add one — e.g. “track our TODO count” or “chart
              bundle size.” It wires up the measure, gauge, and metric in{" "}
              <code>.oxplow/project.yaml</code> and verifies it charts here (the{" "}
              <code>/oxplow:new-metric</code> skill).
            </span>
          </div>
        </div>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
        {loading ? (
          <div style={{ opacity: 0.6 }}>Loading…</div>
        ) : rows.length === 0 ? (
          <div style={{ opacity: 0.6, lineHeight: 1.6 }}>
            No metrics recorded yet. Run tests, coverage, or static analysis —
            oxplow records them into the substrate automatically. For a custom
            metric, ask your agent (the <code>/oxplow:new-metric</code> skill).
          </div>
        ) : sections.length === 0 ? (
          // The Show mode + search can empty the list even though metrics exist,
          // which the "nothing recorded yet" state above doesn't cover.
          <div data-testid="recorded-no-match" style={{ opacity: 0.6, lineHeight: 1.6 }}>
            {showMode === "off-target"
              ? query
                ? "No matching metrics are off target."
                : "No metrics are off target — everything with a target is meeting it."
              : "No metrics match."}
            {showMode === "enabled" ? " Try Show: All to include metrics this project hasn't enabled." : ""}
          </div>
        ) : (
          // A section only exists when it has rows — `buildMetricSections` groups
          // what it's given, so filtering a category empty removes its heading too.
          <>
            {sections.map((group) => (
              <CollapsibleSection
                key={group.key}
                id={group.key}
                title={group.label}
                count={group.entries.length}
              >
                <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13, tableLayout: "fixed" }}>
                  <colgroup>
                    <col />
                    {/* sparkline, then the value that terminates it */}
                    <col style={{ width: 120 }} />
                    <col style={{ width: 140 }} />
                  </colgroup>
                  <tbody>
                    {group.entries.map((row) => (
                      <RecordedRow
                        key={row.key}
                        row={row}
                        stat={lineStat}
                        onOpenPage={onOpenPage}
                        siblings={{
                          entries: siblings.entries,
                          index: siblings.indexByKey.get(row.key) ?? 0,
                          title: "Recorded Metrics",
                        }}
                      />
                    ))}
                  </tbody>
                </table>
              </CollapsibleSection>
            ))}
          </>
        )}
      </div>
    </Page>
    </CollapsibleSections>
  );
}
