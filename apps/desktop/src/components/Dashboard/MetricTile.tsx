import { useEffect, useMemo, useState } from "react";

import {
  type DashboardItem,
  type MetricSpec,
  type RollupRow,
  type SeriesPoint,
  listMetricSamples,
  metricDimensionRollup,
  subscribeOxplowEvents,
} from "../../api.js";
import { metricRef } from "../../tabs/pageRefs.js";
import type { TabRef } from "../../tabs/tabState.js";
import { TrendChart } from "../../pages/MetricDetail.js";
import {
  type TimeRange,
  branchOptions,
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
import { Sparkline } from "../Sparkline.js";
import { useContextMenu } from "../useRowContextMenu.js";

const SAMPLE_LIMIT = 500;
/** Bars shown on a `bar` tile before truncating — a tile is not the breakdown
 *  page; the metric detail is where the full roll-up lives. */
const BAR_ROWS = 6;

/** Compact number formatting for tile headlines: thousands → `1.2k`, small
 *  fractions keep 2 decimals, integers stay bare. */
function fmt(n: number): string {
  if (!Number.isFinite(n)) return "—";
  const abs = Math.abs(n);
  if (abs >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (abs >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  if (Number.isInteger(n)) return String(n);
  return n.toFixed(abs < 1 ? 3 : 2);
}

const TONE_COLOR: Record<"good" | "bad" | "neutral", string> = {
  good: "var(--success, #3fb950)",
  bad: "var(--danger, #f85149)",
  neutral: "var(--text-muted, #888)",
};

/** Shared card shell for every tile kind — the RailHud inset-card visual. */
export function TileCard({
  testId,
  title,
  onTitleClick,
  onContextMenu,
  children,
  menu,
}: {
  testId: string;
  title: string;
  onTitleClick?: (newTab: boolean) => void;
  onContextMenu?: (e: React.MouseEvent) => void;
  children: React.ReactNode;
  menu?: React.ReactNode;
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
 * One dashboard tile for a `metric` item (tsk141/tsk142, epic tsk138). Four
 * visualizations, chosen by the tile's `viz` option:
 *  - `line` (default) — the shared {@link TrendChart};
 *  - `number` — a big headline (latest value) + a signed delta chip colored by
 *    the spec's `direction`;
 *  - `sparkline` — a bare trend line;
 *  - `bar` — the metric rolled up by a dimension (`dim`, default `package`).
 *
 * Samples are windowed by {@link resolveTileWindow} — the dashboard's
 * range/branch filter, with any per-tile override winning. (A `bar` tile reads
 * the dimension roll-up, which is inherently latest-state, so the time filter
 * doesn't apply to it.) Live-refreshes on `metricSamplesChanged`; the page
 * passes the resolved `def` so the grid shares one definitions fetch.
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
  const [samples, setSamples] = useState<SeriesPoint[]>([]);
  const [rollup, setRollup] = useState<RollupRow[]>([]);
  const [loading, setLoading] = useState(true);
  const ctxMenu = useContextMenu();

  const metricKey = item.metric_key ?? null;
  const viz = opts.viz ?? "line";
  const dim = opts.dim ?? "package";

  useEffect(() => {
    if (!metricKey) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    const refresh = () => {
      void listMetricSamples(metricKey, SAMPLE_LIMIT).then((rows) => {
        if (cancelled) return;
        setSamples(rows);
        setLoading(false);
        // Feed the dashboard's branch filter its options (union across tiles).
        onBranches?.(branchOptions(rows));
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
    // Deliberately keyed on `metricKey` alone: `onBranches` is a report-upward
    // callback, and depending on it would re-fetch whenever the page re-renders.
  }, [metricKey]);

  // A `bar` tile reads the dimension roll-up rather than the time series.
  useEffect(() => {
    if (!metricKey || viz !== "bar") {
      setRollup([]);
      return;
    }
    let cancelled = false;
    const refresh = () => {
      void metricDimensionRollup(metricKey, dim).then((rows) => {
        if (!cancelled) setRollup(rows);
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
  }, [metricKey, viz, dim]);

  const title = opts.title ?? def?.title ?? metricKey ?? "Metric";

  // The tile's effective window: dashboard filter unless the tile overrides.
  const windowed = useMemo(() => {
    const { range, branch } = resolveTileWindow(opts, dashboard, Date.now());
    const byRange = range ? filterByRange(samples, range) : samples;
    return filterByBranch(byRange, branch);
  }, [opts, dashboard, samples]);

  const openDetail = (newTab?: boolean) => {
    if (metricKey && onOpenPage) onOpenPage(metricRef(metricKey), newTab ? { newTab: true } : undefined);
  };

  const menuItems: MenuItem[] = [
    {
      id: "viz",
      label: "Visualization",
      enabled: !!onConfigure,
      submenu: (["line", "number", "sparkline", "bar"] as const).map((v) => ({
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
      submenu: (["small", "wide", "tall"] as const).map((s) => ({
        id: `size:${s}`,
        label: s[0]!.toUpperCase() + s.slice(1),
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

  const body = (() => {
    if (!metricKey) return <div style={{ opacity: 0.6, fontSize: 13 }}>No metric selected.</div>;
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
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }} data-testid="metric-tile-number">
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

    if (viz === "sparkline") {
      const values = seriesPoints(windowed).map((p) => p.v);
      const value = latestValue(windowed);
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }} data-testid="metric-tile-sparkline">
          <div style={{ fontSize: 20, fontWeight: 600 }}>
            {value != null ? fmt(value) : "—"}
            {def.unit ? <span style={{ fontSize: 12, opacity: 0.6, marginLeft: 4 }}>{def.unit}</span> : null}
          </div>
          <Sparkline values={values} responsive width={240} height={40} />
        </div>
      );
    }

    if (viz === "bar") {
      const rows = rollup.slice(0, BAR_ROWS);
      if (rows.length === 0)
        return (
          <div style={{ opacity: 0.6, fontSize: 13 }} data-testid="metric-tile-bar-empty">
            No {dim} breakdown for this metric.
          </div>
        );
      const max = Math.max(...rows.map((r) => r.value)) || 1;
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }} data-testid="metric-tile-bar">
          {rows.map((r) => (
            <div key={r.key} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
              <span
                style={{
                  width: "38%",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  opacity: 0.8,
                }}
                title={r.key}
              >
                {r.key}
              </span>
              <span style={{ flex: 1, background: "var(--border, #2a2a2a)", borderRadius: 3, height: 10 }}>
                <span
                  style={{
                    display: "block",
                    width: `${(r.value / max) * 100}%`,
                    background: "var(--accent, #58a6ff)",
                    height: 10,
                    borderRadius: 3,
                  }}
                />
              </span>
              <span style={{ width: 52, textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
                {fmt(r.value)}
              </span>
            </div>
          ))}
        </div>
      );
    }

    // line (default)
    const mode = opts.mode ?? "value";
    const points = transformSeries(seriesPoints(windowed), mode);
    return (
      <div data-testid="metric-tile-line">
        <TrendChart
          points={points}
          target={mode === "value" ? def.target : null}
          unit={def.unit}
          scale={opts.scale ?? "auto"}
        />
      </div>
    );
  })();

  return (
    <TileCard
      testId={`metric-tile-${item.id}`}
      title={title}
      onTitleClick={metricKey ? (newTab) => openDetail(newTab) : undefined}
      onContextMenu={(e) => ctxMenu.open(e, menuItems)}
      menu={ctxMenu.menu}
    >
      {body}
    </TileCard>
  );
}
