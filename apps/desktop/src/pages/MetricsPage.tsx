import { useEffect, useState } from "react";

import {
  type MetricDefinition,
  type MetricSample,
  listMetricDefinitions,
  listMetricSamples,
  subscribeOxplowEvents,
} from "../api.js";
import { Card } from "../components/Card.js";
import { fileRef } from "../tabs/pageRefs.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { MetricDetail } from "./MetricDetail.js";
import { MetricsCatalog } from "./MetricsCatalog.js";
import { MetricsExplorer } from "./MetricsExplorer.js";

type Row = {
  def: MetricDefinition;
  latest: MetricSample | null;
  count: number;
  samples: MetricSample[];
};

const SAMPLE_LIMIT = 50;

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
      <polyline
        points={pts}
        fill="none"
        stroke={color ?? "var(--accent, #58a6ff)"}
        strokeWidth={1.5}
      />
    </svg>
  );
}

/** Color a value against the metric's `target`/`fail_at` + `direction` — the
 *  data-driven successor to the hardcoded coverage 50/80 ramp (tsk220). Three
 *  tiers: meets target → ok (green); past the fail floor → fail (red);
 *  in-between (below target, above fail) → warn (amber). `neutral` metrics and
 *  threshold-less metrics are uncolored. */
function statusColor(def: MetricDefinition, value: number): string | undefined {
  if (def.direction === "neutral") return undefined;
  const higher = def.direction === "higher-better";
  const meets = (t: number) => (higher ? value >= t : value <= t);
  const okThreshold = def.target ?? def.warn_at;
  if (okThreshold != null && meets(okThreshold)) return "var(--ok, #3fb950)";
  if (def.fail_at != null && !meets(def.fail_at)) return "var(--err, #f85149)";
  if (okThreshold != null || def.fail_at != null) return "var(--warn, #e5a50a)";
  return undefined;
}

/**
 * Metrics — a first read-only window onto the unified metric substrate
 * (epic tsk213): the catalog of known metric definitions with each metric's
 * latest recorded value, capture branch, and sample count. Live-refreshes on
 * `metricSamplesChanged`. The seed of the full Explorer/Catalog (P4).
 */
export function MetricsPage({
  initialPreset,
  onOpenPage,
}: { initialPreset?: string; onOpenPage?: (ref: TabRef) => void } = {}) {
  const [rows, setRows] = useState<Row[]>([]);
  const [loading, setLoading] = useState(true);
  const [detail, setDetail] = useState<MetricDefinition | null>(null);

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void listMetricDefinitions().then(async (defs) => {
        const built = await Promise.all(
          defs.map(async (def) => {
            const samples = await listMetricSamples(def.key, SAMPLE_LIMIT);
            return { def, latest: samples[0] ?? null, count: samples.length, samples };
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

  return (
    <Page testId="page-metrics" title="Metrics">
      <div
        style={{
          padding: "16px 20px",
          display: "flex",
          flexDirection: "column",
          gap: 16,
          maxWidth: 1000,
        }}
      >
        {detail ? (
          <Card testId="metric-detail-card" title="Metric detail">
            <MetricDetail def={detail} onBack={() => setDetail(null)} />
          </Card>
        ) : null}
        {!detail && !loading && rows.length > 0 ? (
          <Card testId="metrics-explorer-card" title="Explorer">
            <MetricsExplorer
              defs={rows.map((r) => r.def)}
              onOpenDetail={setDetail}
              initialPreset={initialPreset}
            />
          </Card>
        ) : null}
        {!detail ? (
          <Card testId="metrics-catalog-browse-card" title="Catalog — browse + enable">
            <MetricsCatalog
              onOpenScript={onOpenPage ? (path) => onOpenPage(fileRef(path)) : undefined}
            />
          </Card>
        ) : null}
        {!detail ? (
        <Card testId="metrics-catalog-card" title="Recorded metrics">
          {loading ? (
            <div style={{ opacity: 0.6 }}>Loading…</div>
          ) : rows.length === 0 ? (
            <div style={{ opacity: 0.6, lineHeight: 1.6 }}>
              No metrics recorded yet. Run tests, coverage, or static analysis —
              oxplow records them into the substrate automatically. Custom
              metrics can be declared in <code>oxplow.yaml</code>.
            </div>
          ) : (
            <table
              style={{ width: "100%", borderCollapse: "collapse", fontSize: 13 }}
            >
              <thead>
                <tr style={{ textAlign: "left", opacity: 0.6 }}>
                  <th style={{ padding: "4px 8px" }}>Metric</th>
                  <th style={{ padding: "4px 8px" }}>Kind</th>
                  <th style={{ padding: "4px 8px" }}>Latest</th>
                  <th style={{ padding: "4px 8px" }}>Trend</th>
                  <th style={{ padding: "4px 8px" }}>Branch</th>
                  <th style={{ padding: "4px 8px", textAlign: "right" }}>
                    Samples
                  </th>
                </tr>
              </thead>
              <tbody>
                {rows.map(({ def, latest, count, samples }) => (
                  <tr
                    key={def.key}
                    onClick={() => setDetail(def)}
                    style={{
                      borderTop: "1px solid var(--border, #2a2a2a)",
                      cursor: "pointer",
                    }}
                  >
                    <td style={{ padding: "6px 8px" }}>
                      <div style={{ fontWeight: 600 }}>{def.title}</div>
                      <div
                        style={{
                          opacity: 0.5,
                          fontFamily: "monospace",
                          fontSize: 11,
                        }}
                      >
                        {def.key}
                      </div>
                    </td>
                    <td style={{ padding: "6px 8px" }}>{def.kind}</td>
                    <td
                      style={{
                        padding: "6px 8px",
                        fontWeight: 600,
                        color: latest
                          ? statusColor(def, latest.value)
                          : undefined,
                      }}
                    >
                      {latest
                        ? `${formatValue(latest.value)}${
                            def.unit ? ` ${def.unit}` : ""
                          }`
                        : "—"}
                    </td>
                    <td style={{ padding: "6px 8px" }}>
                      <Sparkline
                        values={samples
                          .slice()
                          .reverse()
                          .map((s) => s.value)}
                        color={
                          latest ? statusColor(def, latest.value) : undefined
                        }
                      />
                    </td>
                    <td
                      style={{
                        padding: "6px 8px",
                        fontFamily: "monospace",
                        fontSize: 11,
                      }}
                    >
                      {latest?.branch ?? "—"}
                    </td>
                    <td style={{ padding: "6px 8px", textAlign: "right" }}>
                      {count}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Card>
        ) : null}
      </div>
    </Page>
  );
}
