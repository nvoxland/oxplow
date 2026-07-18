import { useEffect, useMemo, useState } from "react";

import {
  type DashboardItem,
  type MetricSpec,
  type SeriesPoint,
  listMetricSamples,
  subscribeOxplowEvents,
} from "../../api.js";
import { metricRef } from "../../tabs/pageRefs.js";
import type { TabRef } from "../../tabs/tabState.js";
import { TrendChart } from "../../pages/MetricDetail.js";
import {
  deltaVsFirst,
  seriesPoints,
  transformSeries,
} from "../../pages/metricDetailData.js";
import { deltaTone, latestValue, parseTileOptions } from "../../pages/customDashboardData.js";
import { useContextMenu } from "../useRowContextMenu.js";

const SAMPLE_LIMIT = 500;

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

/**
 * One dashboard tile for a `metric` item (tsk141, epic tsk138). Phase 3 renders
 * two visualizations, chosen by the tile's `options_json` `viz` field:
 *  - `line` (default) — the shared {@link TrendChart} over the metric's samples;
 *  - `number` — a big headline (latest value) + a signed delta chip colored by
 *    the spec's `direction`.
 * The tile fetches its own samples and live-refreshes on `metricSamplesChanged`;
 * the page passes the resolved `def` (so the whole grid shares one definitions
 * fetch). Clicking the title drills through to the metric's detail page; right-
 * click opens the tile actions menu (open / open-in-new-tab / remove).
 */
export function MetricTile({
  item,
  def,
  onOpenPage,
  onRemove,
}: {
  item: DashboardItem;
  def: MetricSpec | null;
  onOpenPage?: (ref: TabRef, opts?: { newTab?: boolean }) => void;
  onRemove?: () => void;
}) {
  const opts = useMemo(() => parseTileOptions(item.options_json), [item.options_json]);
  const [samples, setSamples] = useState<SeriesPoint[]>([]);
  const [loading, setLoading] = useState(true);
  const ctxMenu = useContextMenu();

  const metricKey = item.metric_key ?? null;

  useEffect(() => {
    if (!metricKey) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    const refresh = () => {
      void listMetricSamples(metricKey, SAMPLE_LIMIT).then((rows) => {
        if (!cancelled) {
          setSamples(rows);
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
  }, [metricKey]);

  const title = opts.title ?? def?.title ?? metricKey ?? "Metric";
  const viz = opts.viz ?? "line";

  const openDetail = (newTab?: boolean) => {
    if (metricKey && onOpenPage) onOpenPage(metricRef(metricKey), newTab ? { newTab: true } : undefined);
  };

  const menuItems = [
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
      const value = latestValue(samples);
      const delta = deltaVsFirst(samples);
      const tone = delta != null ? deltaTone(delta, def.direction) : "neutral";
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }} data-testid="metric-tile-number">
          <div style={{ fontSize: 34, fontWeight: 700, lineHeight: 1.1 }}>
            {value != null ? fmt(value) : "—"}
            {def.unit ? <span style={{ fontSize: 15, opacity: 0.6, marginLeft: 4 }}>{def.unit}</span> : null}
          </div>
          {delta != null ? (
            <div style={{ fontSize: 13, color: TONE_COLOR[tone] }}>
              {delta > 0 ? "▲" : delta < 0 ? "▼" : "•"} {fmt(Math.abs(delta))} since first
            </div>
          ) : (
            <div style={{ fontSize: 13, opacity: 0.5 }}>{samples.length} sample{samples.length === 1 ? "" : "s"}</div>
          )}
        </div>
      );
    }
    // line (default)
    const mode = opts.mode ?? "value";
    const points = transformSeries(seriesPoints(samples), mode);
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
    <section
      data-testid={`metric-tile-${item.id}`}
      onContextMenu={(e) => ctxMenu.open(e, menuItems)}
      style={{
        background: "var(--surface-card)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 6,
        padding: 12,
        display: "flex",
        flexDirection: "column",
        gap: 10,
        minWidth: 0,
      }}
    >
      <button
        type="button"
        onClick={(e) => openDetail(e.metaKey || e.ctrlKey)}
        onAuxClick={(e) => {
          if (e.button === 1) openDetail(true);
        }}
        disabled={!metricKey}
        title={metricKey ? "Open metric detail" : undefined}
        style={{
          all: "unset",
          cursor: metricKey ? "pointer" : "default",
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
      {body}
      {ctxMenu.menu}
    </section>
  );
}
