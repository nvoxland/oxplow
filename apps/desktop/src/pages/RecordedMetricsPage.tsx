import { useEffect, useMemo, useState } from "react";

import {
  type MetricSpec,
  type SeriesPoint,
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

type Row = {
  def: MetricSpec;
  latest: SeriesPoint | null;
  samples: SeriesPoint[];
};

const SAMPLE_LIMIT = 200;

function formatValue(v: number): string {
  return Number.isInteger(v) ? String(v) : v.toFixed(2);
}

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
 *  data-driven successor to the hardcoded coverage 50/80 ramp (tsk220). */
function statusColor(def: MetricSpec, value: number): string | undefined {
  if (def.direction === "neutral") return undefined;
  const higher = def.direction === "higher-better";
  const meets = (t: number) => (higher ? value >= t : value <= t);
  const okThreshold = def.target ?? def.warn_at;
  if (okThreshold != null && meets(okThreshold)) return "var(--ok, #3fb950)";
  if (def.fail_at != null && !meets(def.fail_at)) return "var(--err, #f85149)";
  if (okThreshold != null || def.fail_at != null) return "var(--warn, #e5a50a)";
  return undefined;
}

/** One metric row: title · latest value · trend sparkline. A `<tr>` that adopts
 *  browser-style click via `useRouteDispatch` (plain → detail in-tab,
 *  modifier/middle/right → new tab). */
function RecordedRow({ row, onOpenPage }: { row: Row; onOpenPage?: (ref: TabRef) => void }) {
  const { def, latest, samples } = row;
  const color = latest ? statusColor(def, latest.value) : undefined;
  const { handlers } = useRouteDispatch(metricRef(def.key), { onNavigate: onOpenPage });
  return (
    <tr
      onClick={handlers.onClick}
      onAuxClick={handlers.onAuxClick}
      onContextMenu={handlers.onContextMenu}
      style={{ borderTop: "1px solid var(--border, #2a2a2a)", cursor: "pointer" }}
    >
      <td style={{ padding: "6px 8px", fontWeight: 600 }}>{def.title}</td>
      <td style={{ padding: "6px 8px" }}>
        <Sparkline
          values={samples
            .slice()
            .reverse()
            .map((s) => s.value)}
          color={color}
        />
      </td>
      {/* The latest value sits AFTER the sparkline because it *is* the
          sparkline's last point — same filtered `samples`, newest first — so the
          chart reads left-to-right into the number that terminates it (tsk82). */}
      <td style={{ padding: "6px 8px", fontWeight: 600, color }}>
        {latest ? `${formatValue(latest.value)}${def.unit ? ` ${def.unit}` : ""}` : "—"}
      </td>
    </tr>
  );
}

const sel = { fontSize: 12, width: "100%" } as const;

/**
 * Recorded Metrics — the seeded definitions as `title · trend sparkline ·
 * latest value` rows, organized as **one table per section** under headings, via
 * the shared `buildMetricSections` (Code gauges / Tests / Coverage / then one
 * top-level section **per language** for static analysis / Operational) — the
 * same sectioning Metric Settings renders, so the two pages can't disagree
 * (tsk81). A right-side panel scopes the latest/trend by a preset time range
 * (default 7 days) and branch. Rows open the per-metric detail page
 * (`metricRef`). Live on `metricSamplesChanged`.
 */
export function RecordedMetricsPage({ onOpenPage }: { onOpenPage?: (ref: TabRef) => void } = {}) {
  const [rows, setRows] = useState<Row[]>([]);
  const [loading, setLoading] = useState(true);
  const [rangeKey, setRangeKey] = useState<string>(DEFAULT_RANGE_KEY);
  const [branch, setBranch] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void listMetricDefinitions().then(async (defs) => {
        const built = await Promise.all(
          defs.map(async (def) => {
            const samples = await listMetricSamples(def.key, SAMPLE_LIMIT);
            return { def, latest: samples[0] ?? null, samples };
          }),
        );
        if (!cancelled) {
          setRows(built);
          setLoading(false);
        }
      });
    };
    refresh();
    const off = subscribeOxplowEvents((e) => {
      if (e.kind === "metricSamplesChanged") refresh();
    });
    return () => {
      cancelled = true;
      off();
    };
  }, []);

  const branches = useMemo(() => branchOptions(rows.flatMap((r) => r.samples)), [rows]);
  // Scope each metric's latest + trend to the selected range + branch. All
  // metrics stay listed; an out-of-scope metric just shows "—".
  const viewRows = useMemo(() => {
    const range = rangeFromPreset(rangeKey, Date.now());
    const q = query.trim().toLowerCase();
    return rows
      .filter((r) => !q || r.def.title.toLowerCase().includes(q) || r.def.key.toLowerCase().includes(q))
      .map((r) => {
        const samples = filterByBranch(filterByRange(r.samples, range), branch);
        return { def: r.def, latest: samples[0] ?? null, samples };
      });
  }, [rows, rangeKey, branch, query]);

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
        </div>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 24 }}>
        {loading ? (
          <div style={{ opacity: 0.6 }}>Loading…</div>
        ) : rows.length === 0 ? (
          <div style={{ opacity: 0.6, lineHeight: 1.6 }}>
            No metrics recorded yet. Run tests, coverage, or static analysis —
            oxplow records them into the substrate automatically. Custom metrics
            can be declared in <code>.oxplow/project.yaml</code>.
          </div>
        ) : (
          <>
            {buildMetricSections(
              viewRows,
              (r) => r.def.category,
              (r) => r.def.language,
            ).map((group) => (
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
                      <RecordedRow key={row.def.key} row={row} onOpenPage={onOpenPage} />
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
