import { useCallback, useEffect, useMemo, useState } from "react";

import { type DashboardItem, type MetricSpec, type SeriesPoint, querySql } from "../../api.js";
import { seriesPoints as seriesPointRows } from "../../metricsSql.js";
import { NO_READS, useRerunOnChange } from "../../lens/lensRerun.js";
import { useRequestGuard } from "../../request-guard.js";
import { formatMetricValue } from "../format.js";
import { metricRef } from "../../tabs/pageRefs.js";
import type { TabRef } from "../../tabs/tabState.js";
import { TrendChart } from "../../pages/MetricDetail.js";
import type { TimeRange } from "../../kit/rangePresets.js";
import {
  branchOptions,
  defaultChartMode,
  deltaVsFirst,
  filterByBranch,
  filterByRange,
  seriesPoints,
  transformSeries,
} from "../../pages/metricDetailData.js";
import {
  type TileOptions,
  deltaTone,
  latestValue,
  resolveTileWindow,
} from "../../pages/customDashboardData.js";
import type { MenuItem } from "../../menu.js";
import { useContextMenu } from "../useRowContextMenu.js";

const SAMPLE_LIMIT = 500;

/** Tile headline numbers go through the SHARED formatter (tsk183). No `unit`
 *  is passed: the tile renders `def.unit` in its own span beside the number. */
function fmt(n: number): string {
  if (!Number.isFinite(n)) return "—";
  return formatMetricValue(n);
}

const TONE_COLOR: Record<"good" | "bad" | "neutral", string> = {
  good: "var(--success, #3fb950)",
  bad: "var(--danger, #f85149)",
  neutral: "var(--text-muted, #888)",
};

/** Card shell for a metric tile — the RailHud inset-card visual. */
export function TileCard({
  testId,
  title,
  onTitleClick,
  onContextMenu,
  children,
  menu,
  minHeight = 240,
}: {
  testId: string;
  title: string;
  onTitleClick?: (newTab: boolean) => void;
  onContextMenu?: (e: React.MouseEvent) => void;
  children: React.ReactNode;
  menu?: React.ReactNode;
  /** Floor for the card. The grid's rows size to content (so a heading band
   *  can be one line tall), so a chart tile asserts its own height here
   *  rather than relying on `gridAutoRows` (tsk147). */
  minHeight?: number;
}) {
  return (
    <section
      data-testid={testId}
      onContextMenu={onContextMenu}
      style={{
        background: "var(--surface-card)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 6,
        padding: 12,
        display: "flex",
        flexDirection: "column",
        gap: 10,
        minWidth: 0,
        height: "100%",
        minHeight,
      }}
    >
      <button
        type="button"
        onClick={(e) => onTitleClick?.(e.metaKey || e.ctrlKey)}
        onAuxClick={(e) => {
          if (e.button === 1) onTitleClick?.(true);
        }}
        disabled={!onTitleClick}
        title={onTitleClick ? "Open metric detail" : undefined}
        style={{
          all: "unset",
          cursor: onTitleClick ? "pointer" : "default",
          fontWeight: 600,
          fontSize: "var(--text-base, 14px)",
          color: "var(--text, #ddd)",
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "ellipsis",
          minWidth: 0,
        }}
      >
        {title}
      </button>
      {children}
      {menu}
    </section>
  );
}

/**
 * The metric card's body: a metric's captures (its pinned SQL, `sql`) as a
 * `line` (the shared {@link TrendChart}, charted the way the metric rolls
 * up) or a `number` (the latest value + a signed delta colored by the
 * spec's `direction`), windowed by `window`. Live-refreshes on what it
 * read. The custom-dashboard page's `MetricTile` and a dashboard lens's
 * row tiles draw it.
 */
export function MetricTileBody({
  sql,
  metricKey,
  viz: shown,
  size,
  def,
  window,
  onBranches,
}: {
  sql: string | null;
  metricKey: string | null;
  viz: string | null;
  size: string | null;
  def: MetricSpec | null;
  window: { range: TimeRange | null; branch: string | null };
  onBranches?: (branches: string[]) => void;
}) {
  const [samples, setSamples] = useState<SeriesPoint[]>([]);
  const [reads, setReads] = useState(NO_READS);
  const [loading, setLoading] = useState(true);
  const viz = shown ?? "line";
  const guard = useRequestGuard();

  const refresh = useCallback(() => {
    if (!sql) {
      setLoading(false);
      return;
    }
    const current = guard.begin();
    // A metric with nothing to grid (no spec, or a formula) shows no data.
    void querySql(sql, [], SAMPLE_LIMIT)
      .then((result) => ({ rows: seriesPointRows(result), reads: result.reads }))
      .catch(() => ({ rows: [] as SeriesPoint[], reads: NO_READS }))
      .then(({ rows, reads: r }) => {
        if (!current()) return;
        setSamples(rows);
        setReads(r);
        setLoading(false);
        // Feed the dashboard's branch filter its options (union across tiles).
        onBranches?.(branchOptions(rows));
      });
    // `onBranches` is a report-upward callback, excluded from deps on purpose.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sql, guard]);

  useEffect(() => {
    refresh();
  }, [refresh]);
  useRerunOnChange(reads, refresh);

  const windowed = useMemo(() => {
    const byRange = window.range ? filterByRange(samples, window.range) : samples;
    return filterByBranch(byRange, window.branch);
  }, [window, samples]);

  if (!metricKey || !sql) return <div style={{ opacity: 0.6, fontSize: 13 }}>No metric selected.</div>;
  if (loading) return <div style={{ opacity: 0.6, fontSize: 13 }}>Loading…</div>;
  if (!def)
    return (
      <div style={{ opacity: 0.6, fontSize: 13 }} data-testid="metric-tile-missing">
        Metric not found or disabled.
      </div>
    );

  if (viz === "number") {
    const value = latestValue(windowed);
    const delta = deltaVsFirst(windowed);
    const tone = delta != null ? deltaTone(delta, def.direction) : "neutral";
    return (
      <div
        style={{ flex: 1, display: "flex", flexDirection: "column", justifyContent: "center", gap: 4 }}
        data-testid="metric-tile-number"
      >
        <div style={{ fontSize: 34, fontWeight: 700, lineHeight: 1.1 }}>
          {value != null ? fmt(value) : "—"}
          {def.unit ? <span style={{ fontSize: 15, opacity: 0.6, marginLeft: 4 }}>{def.unit}</span> : null}
        </div>
        {delta != null ? (
          <div style={{ fontSize: 13, color: TONE_COLOR[tone] }}>
            {delta > 0 ? "▲" : delta < 0 ? "▼" : "•"} {fmt(Math.abs(delta))} in range
          </div>
        ) : (
          <div style={{ fontSize: 13, opacity: 0.5 }}>
            {windowed.length} sample{windowed.length === 1 ? "" : "s"}
          </div>
        )}
      </div>
    );
  }

  const mode = defaultChartMode(def.aggregation);
  const points = transformSeries(seriesPoints(windowed), mode);
  return (
    <div data-testid="metric-tile-line" style={{ flex: 1, display: "flex", alignItems: "center" }}>
      <TrendChart
        points={points}
        target={mode === "value" ? def.target : null}
        unit={def.unit}
        // Sized near the tile's own width so the drawing renders ~1:1 and the
        // 9px tick labels stay readable instead of scaling down (tsk144).
        width={size === "wide" ? 820 : 400}
        height={size === "tall" ? 380 : 200}
      />
    </div>
  );
}

/**
 * One dashboard tile for a `metric` item: the {@link MetricTileBody} in a
 * card titled by the metric, with its menu. Samples are windowed by
 * {@link resolveTileWindow} — the dashboard's range/branch filter, with any
 * per-tile override winning. The page passes the resolved `def` so the grid
 * shares one definitions fetch.
 */
export function MetricTile({
  item,
  opts,
  def,
  dashboard,
  onOpenPage,
  onRemove,
  onConfigure,
  onBranches,
}: {
  item: DashboardItem;
  opts: TileOptions;
  def: MetricSpec | null;
  dashboard: { range: TimeRange | null; branch: string | null };
  onOpenPage?: (ref: TabRef, opts?: { newTab?: boolean }) => void;
  onRemove?: () => void;
  onConfigure?: (next: Partial<TileOptions>) => void;
  onBranches?: (branches: string[]) => void;
}) {
  const ctxMenu = useContextMenu();
  // A query tile shown as the metric card: its pinned SQL reads the metric's
  // captures (`metric_grid('capture')`, P4.7); `metric` names the metric.
  const metricKey = opts.metric ?? null;
  const viz = opts.viz ?? "line";
  const title = opts.title ?? def?.title ?? metricKey ?? "Metric";
  const window = useMemo(() => resolveTileWindow(opts, dashboard, Date.now()), [opts, dashboard]);

  const openDetail = (newTab?: boolean) => {
    if (metricKey && onOpenPage) onOpenPage(metricRef(metricKey), newTab ? { newTab: true } : undefined);
  };

  const menuItems: MenuItem[] = [
    {
      id: "viz",
      label: "Visualization",
      enabled: !!onConfigure,
      submenu: (["line", "number"] as const).map((v) => ({
        id: `viz:${v}`,
        label: v[0]!.toUpperCase() + v.slice(1),
        enabled: true,
        checked: viz === v,
        run: () => onConfigure?.({ viz: v }),
      })),
    },
    {
      id: "size",
      label: "Size",
      enabled: !!onConfigure,
      submenu: (
        [
          ["small", "Small"],
          ["wide", "Wide (2 columns)"],
          ["tall", "Tall (2 rows)"],
          ["full", "Full width"],
        ] as const
      ).map(([s, label]) => ({
        id: `size:${s}`,
        label,
        enabled: true,
        checked: (opts.size ?? "small") === s,
        run: () => onConfigure?.({ size: s }),
      })),
    },
    { id: "sep", label: "", enabled: false, separator: true },
    { id: "open", label: "Open metric detail", enabled: !!metricKey, run: () => openDetail() },
    { id: "open-new", label: "Open in new tab", enabled: !!metricKey, run: () => openDetail(true) },
    { id: "remove", label: "Remove from dashboard", enabled: !!onRemove, run: () => onRemove?.() },
  ];

  return (
    <TileCard
      testId={`metric-tile-${item.id}`}
      title={title}
      onTitleClick={metricKey ? (newTab) => openDetail(newTab) : undefined}
      onContextMenu={(e) => ctxMenu.open(e, menuItems)}
      menu={ctxMenu.menu}
      // `tall` asks for twice the height; with content-sized rows the tile
      // states that directly rather than leaning on the row track.
      minHeight={opts.size === "tall" ? 500 : 240}
    >
      <MetricTileBody sql={opts.sql ?? null} metricKey={metricKey} viz={opts.viz ?? null} size={opts.size ?? null} def={def} window={window} onBranches={onBranches} />
    </TileCard>
  );
}
