import { EmptyState } from "../components/Prompts/EmptyState.js";
import { useCallback, useEffect, useMemo, useState } from "react";
import { formatMetricValue, formatMetricValueExact } from "../components/format";

import {
  type MetricSpec,
  type SeriesPoint,
  listMetricCatalog,
  listMetricDefinitions,
  listMetricSamples,
} from "../api.js";
import { coalescedRefresh } from "../coalesced-refresh.js";
import { NO_READS, unionReads, useRerunOnChange } from "../lens/lensRerun.js";
import { useRequestGuard } from "../request-guard.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";
import {
  CollapsibleSection,
  CollapsibleSections,
  SectionCollapseControls,
} from "../components/CollapsibleSections.js";
import { Sparkline } from "../components/Sparkline.js";
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
  widestPresetWindow,
} from "./metricDetailData.js";
import {
  DEFAULT_SHOW_MODE,
  SHOW_MODES,
  type ShowMode,
  filterMetricRows,
  metricStatusColor,
  metricSiblings,
} from "./metricsRows.js";

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



/** One metric row: title · trend sparkline · latest value. A `<tr>`
 *  that adopts browser-style click via `useRouteDispatch` (plain → detail
 *  in-tab, modifier/middle/right → new tab). */
function MetricRow({
  row,
  onOpenPage,
  siblings,
}: {
  row: Row;
  onOpenPage?: (ref: TabRef) => void;
  siblings?: NavSiblings;
}) {
  const { def, latest, samples } = row;
  // Colored by where the latest value stands against the metric's target.
  const color = def && latest ? metricStatusColor(def, latest.value) : undefined;
  const unit = def?.unit;
  const { handlers } = useRouteDispatch(metricRef(row.key), { onNavigate: onOpenPage, siblings });
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
        style={{ padding: "6px 8px", fontWeight: 600, color }}
        title={latest ? formatMetricValueExact(latest.value, unit) : undefined}
      >
        {latest ? formatMetricValue(latest.value, unit) : "—"}
      </td>
    </tr>
  );
}

const sel = { fontSize: 12, width: "100%" } as const;

/**
 * Metrics — every catalogued metric as a `title · trend sparkline ·
 * latest value` row, organized as **one table per section** under headings, via
 * the shared `buildMetricSections` (Code metrics / Tests / Coverage / then one
 * top-level section **per language** for static analysis / Operational)
 * (tsk81). A right-side panel scopes the latest/trend by a preset time range
 * (default 7 days) and branch, picks Enabled (default) / All, and holds the
 * Expand/Collapse-all controls. Rows open the per-metric detail page
 * (`metricRef`), which is also where a metric is enabled/disabled and its
 * target set (tsk117). Authoring a NEW custom metric is agent work now (the
 * "+ New metric" scaffold form was retired in tsk122 for agent-driven authoring
 * via the `/oxplow:new-metric` skill + the `metric.scaffold` command); the rail
 * carries a Help blurb pointing there.
 * Live on `metricSamplesChanged` (debounced) and `configChanged`.
 *
 * **The row set is the CATALOG, not the spec table (tsk87).** Only the catalog
 * knows about `use:`: a built-in metric keeps its seeded spec when merely
 * un-`use:`d (its collector just never runs), so reading specs alone listed the
 * bundled C#/Clojure idiom metrics in a Rust/TS repo as permanent `—` rows while Metric
 * Settings showed the same rows unchecked. The spec joins in by key for the
 * presentation metadata (unit / direction / thresholds) and is null only for an
 * explicitly disabled metric, whose spec is pruned.
 */
export function MetricsPage({ onOpenPage }: { onOpenPage?: (ref: TabRef) => void } = {}) {
  const [rows, setRows] = useState<Row[]>([]);
  const [loading, setLoading] = useState(true);
  const [rangeKey, setRangeKey] = useState<string>(DEFAULT_RANGE_KEY);
  const [branch, setBranch] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [showMode, setShowMode] = useState<ShowMode>(DEFAULT_SHOW_MODE);

  const [reads, setReads] = useState<Reads>(NO_READS);
  const guard = useRequestGuard();

  // One read per catalogued metric. It re-runs when something those reads
  // read changed: the catalog or a spec (an enable toggle, a project.yaml
  // edit) or facts for any listed metric's measure (P4.6/P4.7).
  // Behind a single-flight gate: the facts for token measures land every
  // agent turn, and a refresh must never overlap the one before it (tsk91).
  const load = useCallback(() => {
    const current = guard.begin();
    return Promise.all([listMetricCatalog(), listMetricDefinitions()]).then(async ([catalog, defs]) => {
      const specs = new Map(defs.rows.map((d) => [d.key, d]));
      const built = await Promise.all(
        catalog.rows.map(async (entry) => {
          const def = specs.get(entry.key) ?? null;
          // No spec ⇒ nothing to grid. Bound the read to the widest preset
          // (tsk202); the page filters to the selected range client-side.
          const samples = def
            ? await listMetricSamples(entry.key, SAMPLE_LIMIT, null, widestPresetWindow(Date.now()))
            : { rows: [], reads: NO_READS };
          const row: Row = {
            key: entry.key,
            title: entry.title,
            category: entry.category,
            language: entry.language,
            enabled: entry.enabled,
            def,
            latest: samples.rows[0] ?? null,
            samples: samples.rows,
          };
          return { row, reads: samples.reads };
        }),
      );
      if (!current()) return;
      setRows(built.map((b) => b.row));
      setReads(unionReads([catalog.reads, defs.reads, ...built.map((b) => b.reads)]));
      setLoading(false);
    });
  }, [guard]);
  const gate = useMemo(() => coalescedRefresh(load, 0), [load]);
  useEffect(() => {
    gate.schedule();
    return () => gate.cancel();
  }, [gate]);
  useRerunOnChange(reads, gate.schedule);

  const branches = useMemo(() => branchOptions(rows.flatMap((r) => r.samples)), [rows]);
  // Which metrics are LISTED (Show mode + search), each scoped to the range +
  // branch — an in-scope metric with no recording in the window stays listed
  // and just shows "—".
  const viewRows = useMemo(() => {
    const range = rangeFromPreset(rangeKey, Date.now());
    return filterMetricRows(rows, showMode, query).map((r) => {
      const samples = filterByBranch(filterByRange(r.samples, range), branch);
      return { ...r, latest: samples[0] ?? null, samples };
    });
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
      title="Metrics"
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
              title="Enabled lists only metrics this project has turned on; All also lists the ones it hasn't."
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
          {/* Not a filter, but this rail is the page's control panel — the
              controls self-hide while there are no sections to act on. */}
          <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
            <SectionCollapseControls />
          </div>
          {/* Authoring a custom metric is agent work now (the "+ New metric"
              scaffold form was retired, tsk122): the agent wires up the trio +
              collector script correctly via the /oxplow:new-metric skill. This blurb
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
              bundle size.” It wires up the measure, collector, and metric in{" "}
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
          <EmptyState
            testId="recorded-empty"
            title="No metrics recorded yet"
            text="Run tests, coverage or static analysis and oxplow records them here automatically."
            prompts={["Add a metric that tracks how many TODO comments this project has"]}
          />
        ) : sections.length === 0 ? (
          // The Show mode + search can empty the list even though metrics exist,
          // which the "nothing recorded yet" state above doesn't cover.
          <EmptyState
            testId="recorded-no-match"
            title="No metrics match"
            text={showMode === "enabled" ? "Try Show: All to include metrics this project hasn't enabled." : "Clear the search to see them all."}
          />
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
                      <MetricRow
                        key={row.key}
                        row={row}
                        onOpenPage={onOpenPage}
                        siblings={{
                          entries: siblings.entries,
                          index: siblings.indexByKey.get(row.key) ?? 0,
                          title: "Metrics",
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
