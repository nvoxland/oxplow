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
import { metricStatus, metricStatusColor } from "../../pages/recordedMetricsRows.js";
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
  minHeight = 240,
  alertColor,
  alertLabel,
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
  /** When set, the card is off target: its border takes this color and a chip
   *  reading {@link alertLabel} sits beside the title (tsk149). */
  alertColor?: string;
  alertLabel?: string;
}) {
  return (
    <section
      data-testid={testId}
      onContextMenu={onContextMenu}
      style={{
        background: "var(--surface-card)",
        border: `1px solid ${alertColor ?? "var(--border-subtle)"}`,
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
      <div style={{ display: "flex", alignItems: "center", gap: 8, minWidth: 0 }}>
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
        {alertColor && alertLabel ? (
          <span
            data-testid="tile-off-target"
            title="This metric is missing its target"
            style={{
              flexShrink: 0,
              fontSize: 10,
              fontWeight: 700,
              letterSpacing: "0.04em",
              textTransform: "uppercase",
              color: alertColor,
              border: `1px solid ${alertColor}`,
              borderRadius: 4,
              padding: "1px 5px",
            }}
          >
            {alertLabel}
          </span>
        ) : null}
      </div>
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

  // Off-target highlight (tsk149). Uses the project's ONE classifier
  // (`metricStatus`) and its shared color mapping, so a tile can't disagree
  // with the Recorded Metrics page or its Off-target filter about a verdict.
  // Defaults on: it only fires for a metric that HAS a target and is missing
  // it, so it stays silent for the many metrics with no target at all.
  const alertEnabled = opts.alertOffTarget ?? true;
  const currentValue = latestValue(windowed);
  const status = def && currentValue != null ? metricStatus(def, currentValue) : "none";
  const offTarget = alertEnabled && (status === "warn" || status === "fail");
  const alertColor =
    offTarget && def && currentValue != null ? metricStatusColor(def, currentValue) : undefined;

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
    {
      id: "alert",
      label: "Warn when off target",
      enabled: !!onConfigure,
      checked: alertEnabled,
      run: () => onConfigure?.({ alertOffTarget: !alertEnabled }),
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
        <div
          style={{ flex: 1, display: "flex", flexDirection: "column", justifyContent: "center", gap: 4 }}
          data-testid="metric-tile-number"
        >
          <div style={{ fontSize: 34, fontWeight: 700, lineHeight: 1.1, color: alertColor }}>
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
        <div
          style={{ flex: 1, display: "flex", flexDirection: "column", justifyContent: "center", gap: 8 }}
          data-testid="metric-tile-sparkline"
        >
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
      <div data-testid="metric-tile-line" style={{ flex: 1, display: "flex", alignItems: "center" }}>
        <TrendChart
          points={points}
          target={mode === "value" ? def.target : null}
          unit={def.unit}
          scale={opts.scale ?? "auto"}
          // Sized near the tile's own width so the drawing renders ~1:1 and the
          // 9px tick labels stay readable instead of scaling down (tsk144).
          width={opts.size === "wide" ? 820 : 400}
          height={opts.size === "tall" ? 380 : 200}
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
      // `tall` asks for twice the height; with content-sized rows the tile
      // states that directly rather than leaning on the row track.
      minHeight={opts.size === "tall" ? 500 : 240}
      alertColor={alertColor}
      // Direction-agnostic wording: "below" would be wrong for a lower-better
      // metric, where missing the target means being ABOVE it.
      alertLabel={offTarget ? (status === "fail" ? "Failing" : "Off target") : undefined}
    >
      {body}
    </TileCard>
  );
}
