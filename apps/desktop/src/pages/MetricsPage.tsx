import { useEffect, useState } from "react";

import {
  type MetricDefinition,
  type MetricSample,
  listMetricDefinitions,
  listMetricSamples,
  subscribeOxplowEvents,
} from "../api.js";
import { Card } from "../components/Card.js";
import { Page } from "../tabs/Page.js";

type Row = { def: MetricDefinition; latest: MetricSample | null; count: number };

const SAMPLE_LIMIT = 50;

function formatValue(v: number): string {
  return Number.isInteger(v) ? String(v) : v.toFixed(2);
}

/** Color a value against the metric's target + direction (the data-driven
 *  successor to the hardcoded coverage ramps). */
function statusColor(def: MetricDefinition, value: number): string | undefined {
  const threshold = def.fail_at ?? def.warn_at ?? def.target;
  if (threshold == null || def.direction === "neutral") return undefined;
  const ok =
    def.direction === "higher-better" ? value >= threshold : value <= threshold;
  return ok ? "var(--ok, #3fb950)" : "var(--warn, #e5a50a)";
}

/**
 * Metrics — a first read-only window onto the unified metric substrate
 * (epic tsk213): the catalog of known metric definitions with each metric's
 * latest recorded value, capture branch, and sample count. Live-refreshes on
 * `metricSamplesChanged`. The seed of the full Explorer/Catalog (P4).
 */
export function MetricsPage() {
  const [rows, setRows] = useState<Row[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void listMetricDefinitions().then(async (defs) => {
        const built = await Promise.all(
          defs.map(async (def) => {
            const samples = await listMetricSamples(def.key, SAMPLE_LIMIT);
            return { def, latest: samples[0] ?? null, count: samples.length };
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
        <Card testId="metrics-catalog-card" title="Metric catalog">
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
                  <th style={{ padding: "4px 8px" }}>Branch</th>
                  <th style={{ padding: "4px 8px", textAlign: "right" }}>
                    Samples
                  </th>
                </tr>
              </thead>
              <tbody>
                {rows.map(({ def, latest, count }) => (
                  <tr
                    key={def.key}
                    style={{ borderTop: "1px solid var(--border, #2a2a2a)" }}
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
      </div>
    </Page>
  );
}
