import { useEffect, useState } from "react";

import {
  type MetricDefinition,
  type MetricFinding,
  listMetricDefinitions,
  listMetricFindings,
} from "../api.js";
import { Page } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";

function fmt(v: number): string {
  return Number.isInteger(v) ? String(v) : v.toFixed(2);
}

/**
 * Metric Recording — the located items a "count of X" gauge counted at one
 * recording (run), so you can drill from a metric's trend to the actual
 * contributing functions/lines (tsk313). Opened from a row in the Metric Detail
 * recordings table (`metricRecordingRef(runId, …)`); reads `metric_finding`s by
 * run id. Pure read; degrades gracefully when a run has no findings (e.g. a
 * gauge that doesn't emit them).
 */
export function MetricRecordingPage({
  runId,
  metricKey,
  capturedAt,
  value,
}: {
  runId?: number;
  metricKey?: string;
  capturedAt?: string;
  value?: number;
  onOpenPage?: (ref: TabRef) => void;
} = {}) {
  const [findings, setFindings] = useState<MetricFinding[]>([]);
  const [def, setDef] = useState<MetricDefinition | null>(null);
  const [loading, setLoading] = useState(true);
  const title = def?.title ?? "Recording";
  usePageTitle(title);

  useEffect(() => {
    let cancelled = false;
    if (runId == null) {
      setLoading(false);
      return;
    }
    void listMetricFindings(runId).then((rows) => {
      if (!cancelled) {
        setFindings(rows);
        setLoading(false);
      }
    });
    if (metricKey) {
      void listMetricDefinitions().then((defs) => {
        if (!cancelled) setDef(defs.find((d) => d.key === metricKey) ?? null);
      });
    }
    return () => {
      cancelled = true;
    };
  }, [runId, metricKey]);

  // Sort by descending value (worst first), then path/line.
  const rows = findings
    .slice()
    .sort(
      (a, b) =>
        (b.value ?? 0) - (a.value ?? 0) ||
        (a.path ?? "").localeCompare(b.path ?? "") ||
        (a.start_line ?? 0) - (b.start_line ?? 0),
    );

  return (
    <Page testId="page-metric-recording" title={title}>
      <div style={{ padding: "16px 20px", maxWidth: 1000, display: "flex", flexDirection: "column", gap: 16 }}>
        <div style={{ display: "flex", gap: 16, fontSize: 13, opacity: 0.75, flexWrap: "wrap" }}>
          {capturedAt ? <span>Recorded {new Date(String(capturedAt)).toLocaleString()}</span> : null}
          {value != null ? (
            <span>
              Value <strong>{fmt(value)}{def?.unit ? ` ${def.unit}` : ""}</strong>
            </span>
          ) : null}
          <span>
            {findings.length} {findings.length === 1 ? "item" : "items"}
          </span>
        </div>
        {loading ? (
          <div style={{ opacity: 0.6 }}>Loading…</div>
        ) : rows.length === 0 ? (
          <div style={{ opacity: 0.6, lineHeight: 1.6 }}>
            This recording has no item-level detail. (Not every metric records the
            individual items it counts.)
          </div>
        ) : (
          <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13 }} data-testid="recording-findings">
            <thead>
              <tr style={{ textAlign: "left", opacity: 0.6 }}>
                <th style={{ padding: "4px 8px" }}>Item</th>
                <th style={{ padding: "4px 8px" }}>Location</th>
                <th style={{ padding: "4px 8px", textAlign: "right" }}>Value</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((f) => (
                <tr key={f.id} style={{ borderTop: "1px solid var(--border, #2a2a2a)" }}>
                  <td style={{ padding: "6px 8px", fontWeight: 600 }}>{f.message ?? f.subject_ref ?? "—"}</td>
                  <td style={{ padding: "6px 8px", fontFamily: "monospace", fontSize: 11 }}>
                    {f.path ?? "—"}
                    {f.start_line != null ? `:${f.start_line}` : ""}
                  </td>
                  <td style={{ padding: "6px 8px", textAlign: "right" }}>{f.value != null ? fmt(f.value) : "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </Page>
  );
}
