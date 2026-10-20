import { EmptyState } from "../components/Prompts/EmptyState.js";
import { useCallback, useEffect, useMemo, useState } from "react";
import { formatMetricValue, formatMetricValueExact } from "../components/format";

import {
  type MetricSpec,
  type SeriesPoint,
  listMetricCatalog,
  listMetricDefinitions,
  listMetricSamples,
  querySql,
} from "../api.js";
import { coalescedRefresh } from "../coalesced-refresh.js";
import { NO_READS, unionReads, useRerunOnChange } from "../lens/lensRerun.js";
import { useRequestGuard } from "../request-guard.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";
import {
  CollapsibleSections,
  SectionCollapseControls,
} from "../components/CollapsibleSections.js";
import { Sparkline } from "../components/Sparkline.js";
import { metricRef } from "../tabs/pageRefs.js";
import { Page } from "../tabs/Page.js";
import { routeDispatch, routeHandlers } from "../tabs/RouteLink.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { DataTable, MetricFilters, Section, Toolbar, useSearchFilter, type DataColumn } from "../kit/index.js";
import type { OriginItem } from "../tabs/session.js";
import type { TabRef } from "../tabs/tabState.js";
import { buildMetricSections } from "./metricCategories.js";
import { DIMENSIONS_SQL } from "./exploreData.js";
import { DEFAULT_RANGE_KEY, resolveRange } from "../kit/rangePresets.js";
import { useViewParam } from "../tabs/useViewParam.js";
import {
  branchOptions,
  filterByBranch,
  filterByRange,
  widestPresetWindow,
} from "./metricDetailData.js";
import {
  DEFAULT_SHOW_MODE,
  SHOW_MODES,
  type ShowMode,
  filterMetricRows,
  metricStatusColor,
  metricOrigin,
  sliceGroups,
} from "./metricsRows.js";

/** One listed metric. Identity/enabled/grouping come from the **catalog** (the
 *  only source that knows about `use:`); `def` is the seeded spec, which carries
 *  the presentation metadata (unit, direction, thresholds) and is null when the
 *  metric was explicitly disabled and its spec pruned. */
export type Row = {
  key: string;
  title: string;
  category: string | null;
  language: string | null;
  enabled: boolean;
  def: MetricSpec | null;
  latest: SeriesPoint | null;
  samples: SeriesPoint[];
  /** The samples sliced by the chosen dimension (one point per group per
   *  capture); empty when the list isn't sliced or the metric has none. */
  grouped: SeriesPoint[];
};

const SAMPLE_LIMIT = 200;
/** How many groups a sliced metric lists beneath it; the rest are counted. */
const SLICE_TOP_N = 3;



/** One line of the metrics table: a metric (opens its page), or, when the
 *  list is sliced, beneath it a line per top group, a note that it isn't
 *  sliced, or a count of the groups left out. */
export type Line =
  | { kind: "metric"; key: string; row: Row }
  | { kind: "none"; key: string; slice: string }
  | { kind: "group"; key: string; group: string; samples: SeriesPoint[]; latest: SeriesPoint; color?: string; unit?: string | null }
  | { kind: "more"; key: string; hidden: number };

/** A metric's lines: itself, then its slices. */
export function metricLines(row: Row, slice: string | null): Line[] {
  const { def, latest } = row;
  const color = def && latest ? metricStatusColor(def, latest.value) : undefined;
  const { shown, hidden } = sliceGroups(row.grouped, SLICE_TOP_N);
  return [
    { kind: "metric", key: row.key, row },
    // A group is a value of the dimension, not a metric, so it opens no page.
    ...(slice && def && shown.length === 0 ? [{ kind: "none" as const, key: `${row.key}/none`, slice }] : []),
    ...shown.map((g) => ({
      kind: "group" as const,
      key: `${row.key}/${g.group}`,
      group: g.group,
      samples: g.samples,
      latest: g.latest,
      color,
      unit: def?.unit,
    })),
    ...(hidden > 0 ? [{ kind: "more" as const, key: `${row.key}/more`, hidden }] : []),
  ];
}

const trend = (samples: SeriesPoint[], color?: string) => (
  <Sparkline
    values={samples
      .slice()
      .reverse()
      .map((s) => s.value)}
    color={color}
  />
);

const metricColumns: DataColumn<Line>[] = [
  {
    key: "title",
    label: "Metric",
    render: (l) => {
      switch (l.kind) {
        case "metric":
          return <strong>{l.row.title}</strong>;
        case "none":
          return <span className="ox-muted">Not sliced by {l.slice}</span>;
        case "group":
          return <span className="ox-muted">{l.group}</span>;
        case "more":
          return <span className="ox-muted">+ {l.hidden} more</span>;
      }
    },
  },
  {
    key: "trend",
    label: "Trend",
    width: 120,
    render: (l) => {
      if (l.kind === "metric") {
        const color = l.row.def && l.row.latest ? metricStatusColor(l.row.def, l.row.latest.value) : undefined;
        return trend(l.row.samples, color);
      }
      return l.kind === "group" ? trend(l.samples, l.color) : null;
    },
  },
  {
    key: "value",
    label: "Latest",
    width: 140,
    render: (l) => {
      if (l.kind === "metric") {
        const { def, latest } = l.row;
        const color = def && latest ? metricStatusColor(def, latest.value) : undefined;
        return <strong style={{ color }}>{latest ? formatMetricValue(latest.value, def?.unit) : "—"}</strong>;
      }
      return l.kind === "group" ? formatMetricValue(l.latest.value, l.unit) : null;
    },
    title: (l) =>
      l.kind === "metric"
        ? l.row.latest
          ? formatMetricValueExact(l.row.latest.value, l.row.def?.unit)
          : undefined
        : l.kind === "group"
          ? formatMetricValueExact(l.latest.value, l.unit)
          : undefined,
  },
];

/** One group of the metrics list as a table: a row opens the metric's page
 *  (browser-style: plain → in this tab, modifier/middle/right → a new one). */
function MetricsTable({
  entries,
  slice,
  onOpenPage,
  origin,
}: {
  entries: Row[];
  slice: string | null;
  onOpenPage?: (ref: TabRef) => void;
  origin: { items: OriginItem[]; indexByKey: ReadonlyMap<string, number> };
}) {
  const ctxNav = useOptionalPageNavigation();
  const lines = useMemo(() => entries.flatMap((row) => metricLines(row, slice)), [entries, slice]);
  return (
    <DataTable
      header={false}
      columns={metricColumns}
      rows={lines}
      rowKey={(l) => l.key}
      rowProps={(l) => {
        if (l.kind === "metric") {
          const handlers = routeHandlers(
            routeDispatch(metricRef(l.row.key), ctxNav, {
              onNavigate: onOpenPage,
              origin: {
                items: origin.items,
                index: origin.indexByKey.get(l.row.key) ?? 0,
                title: "Metrics",
              },
            }),
          );
          return { className: "ox-clickable", ...handlers };
        }
        const testId = { none: "recorded-slice-none", group: "recorded-slice-group", more: "recorded-slice-more" }[l.kind];
        return { className: "ox-inert", "data-testid": testId };
      }}
    />
  );
}

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
 * via the `/oxplow:new-metric` skill + the `oxplow.metric.scaffold` command); the rail
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
  // The page's filters are its view params (`metricsView.ts`): in its tab's
  // address, so Back and a restart bring them back.
  const [rangeKey, setRangeKey] = useViewParam<string>("range", DEFAULT_RANGE_KEY);
  const [branchName, setBranch] = useViewParam<string>("branch", "");
  const branch = branchName || null;
  const search = useSearchFilter("q", { label: "Filter metrics by name", placeholder: "Filter by name…", testId: "recorded-search" });
  const query = search.value;
  const [showText, setShowMode] = useViewParam("show", DEFAULT_SHOW_MODE as string);
  const showMode: ShowMode = showText === "all" ? "all" : "enabled";
  // Slice By (tsk594): a dimension splits every metric that has it into groups.
  const [sliceKey, setSlice] = useViewParam<string>("slice", "");
  const sliceBy = sliceKey || null;
  const [dimensions, setDimensions] = useState<{ key: string; label: string }[]>([]);
  useEffect(() => {
    void querySql(DIMENSIONS_SQL, [], null)
      .then((r) => setDimensions(r.rows.map(([key, label]) => ({ key: String(key), label: String(label ?? key) }))))
      .catch(() => setDimensions([]));
  }, []);

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
          const window = widestPresetWindow(Date.now());
          const samples = def
            ? await listMetricSamples(entry.key, SAMPLE_LIMIT, null, window)
            : { rows: [], reads: NO_READS };
          // Sliced: the same read, split by the dimension. A metric that
          // doesn't have it just comes back with no groups.
          const grouped =
            def && sliceBy
              ? await listMetricSamples(entry.key, SAMPLE_LIMIT, sliceBy, window).catch(() => ({
                  rows: [] as SeriesPoint[],
                  reads: NO_READS,
                }))
              : { rows: [] as SeriesPoint[], reads: NO_READS };
          const row: Row = {
            key: entry.key,
            title: entry.title,
            category: entry.category,
            language: entry.language,
            enabled: entry.enabled,
            def,
            latest: samples.rows[0] ?? null,
            samples: samples.rows,
            grouped: grouped.rows,
          };
          return { row, reads: unionReads([samples.reads, grouped.reads]) };
        }),
      );
      if (!current()) return;
      setRows(built.map((b) => b.row));
      setReads(unionReads([catalog.reads, defs.reads, ...built.map((b) => b.reads)]));
      setLoading(false);
    });
  }, [guard, sliceBy]);
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
    const range = resolveRange(rangeKey, Date.now());
    const within = (s: SeriesPoint[]) => filterByBranch(range ? filterByRange(s, range) : s, branch);
    return filterMetricRows(rows, showMode, query).map((r) => {
      const samples = within(r.samples);
      const grouped = within(r.grouped);
      return { ...r, latest: samples[0] ?? null, samples, grouped };
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
  const origin = useMemo(() => metricOrigin(sections, (key) => metricRef(key)), [sections]);

  return (
    // The provider wraps the whole Page so its context reaches BOTH the details
    // rail (which holds the Expand/Collapse-all controls) and the body (which
    // holds the sections) — `rightRail` is created here but rendered inside
    // Page's subtree, and context follows the render tree.
    <CollapsibleSections testIdPrefix="recorded">
    <Page
      testId="page-metrics-recorded"
      title="Metrics"
      width="reading"
      rightRailTitle="Filters"
      rightRail={
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          <MetricFilters
            range={rangeKey}
            branch={branchName}
            branches={branches}
            slice={sliceKey}
            dimensions={dimensions}
            show={showMode}
            showOptions={SHOW_MODES.map((m) => ({ value: m.key, label: m.label }))}
            onApply={(name, value) =>
              ({ range: setRangeKey, branch: setBranch, slice: setSlice, show: (v: string) => setShowMode(v) })[name](value)
            }
          />
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
      <Toolbar filter={search} />
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
              <Section
                key={group.key}
                id={group.key}
                title={`${group.label} (${group.entries.length})`}
                collapsible
              >
                <MetricsTable
                  entries={group.entries}
                  slice={sliceBy ? (dimensions.find((d) => d.key === sliceBy)?.label ?? sliceBy) : null}
                  onOpenPage={onOpenPage}
                  origin={origin}
                />
              </Section>
            ))}
          </>
        )}
      </div>
    </Page>
    </CollapsibleSections>
  );
}
