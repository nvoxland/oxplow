import { useEffect, useState } from "react";

import {
  type EffortMetricDelta,
  type MetricDefinition,
  type MetricFinding,
  type MetricSample,
  listEffortMetricDeltas,
  listMetricFindings,
  listMetricSamples,
} from "../api.js";
import {
  deltaColor,
  deltaSummary,
  fmtSigned,
} from "../components/EffortMetrics.js";
import {
  deltaVsFirst,
  findingRows,
  parseDetailPayload,
  seriesPoints,
  topSubjects,
} from "./metricDetailData.js";

// Per-kind Metric detail view (tsk213, P4 / tsk232): one renderer selected from
// `metric_definition.kind`. Every kind shares a value trend (read off the same
// `metric_sample` facts); each adds its kind-specific drill-in from the latest
// run's findings — so a new metric of a known kind gets this for free.

const SAMPLE_LIMIT = 200;

function fmt(v: number): string {
  return Number.isInteger(v) ? String(v) : v.toFixed(2);
}

/** Compact single-series trend line — the shared header chart for every kind. */
function TrendChart({ samples, target }: { samples: MetricSample[]; target?: number | null }) {
  const pts = seriesPoints(samples);
  if (pts.length < 2) {
    return <div style={{ opacity: 0.6, padding: 16 }}>Not enough samples to chart yet.</div>;
  }
  const w = 760;
  const h = 200;
  const padL = 40;
  const padR = 12;
  const padT = 10;
  const padB = 22;
  const tMin = Math.min(...pts.map((p) => p.t));
  const tMax = Math.max(...pts.map((p) => p.t));
  const vMin = Math.min(0, ...pts.map((p) => p.v));
  const vMax = Math.max(...pts.map((p) => p.v), target ?? -Infinity);
  const tRange = tMax - tMin || 1;
  const vRange = vMax - vMin || 1;
  const x = (t: number) => padL + ((t - tMin) / tRange) * (w - padL - padR);
  const y = (v: number) => h - padB - ((v - vMin) / vRange) * (h - padT - padB);
  const d = pts.map((p, i) => `${i === 0 ? "M" : "L"}${x(p.t).toFixed(1)},${y(p.v).toFixed(1)}`).join(" ");
  return (
    <svg width={w} height={h} style={{ display: "block", maxWidth: "100%" }} role="img" aria-label="metric trend">
      <line x1={padL} y1={padT} x2={padL} y2={h - padB} stroke="var(--border, #2a2a2a)" />
      <line x1={padL} y1={h - padB} x2={w - padR} y2={h - padB} stroke="var(--border, #2a2a2a)" />
      {[0, 0.5, 1].map((fr) => {
        const v = vMin + fr * (vMax - vMin);
        return (
          <g key={fr}>
            <text x={padL - 6} y={y(v) + 3} textAnchor="end" fontSize={9} fill="var(--text-muted, #888)">
              {Number.isInteger(v) ? v : v.toFixed(1)}
            </text>
            <line x1={padL} y1={y(v)} x2={w - padR} y2={y(v)} stroke="var(--border, #2a2a2a)" opacity={0.3} />
          </g>
        );
      })}
      {target != null ? (
        <line x1={padL} y1={y(target)} x2={w - padR} y2={y(target)} stroke="var(--ok, #3fb950)" strokeDasharray="4 3" opacity={0.7} />
      ) : null}
      <path d={d} fill="none" stroke="var(--accent, #58a6ff)" strokeWidth={1.5} />
      {pts.map((p, i) => (
        <circle key={i} cx={x(p.t)} cy={y(p.v)} r={1.8} fill="var(--accent, #58a6ff)" />
      ))}
    </svg>
  );
}

function FindingsTable({ findings }: { findings: MetricFinding[] }) {
  const rows = findingRows(findings);
  if (rows.length === 0) return <div style={{ opacity: 0.6 }}>No findings in the latest run.</div>;
  return (
    <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
      <thead>
        <tr style={{ textAlign: "left", opacity: 0.6 }}>
          <th style={{ padding: "4px 8px" }}>Location</th>
          <th style={{ padding: "4px 8px" }}>Severity</th>
          <th style={{ padding: "4px 8px" }}>Rule</th>
          <th style={{ padding: "4px 8px" }}>Message</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.id} style={{ borderTop: "1px solid var(--border, #2a2a2a)" }}>
            <td style={{ padding: "4px 8px", fontFamily: "monospace", fontSize: 11 }}>
              {r.path ?? r.subject_ref ?? "—"}
              {r.start_line != null ? `:${r.start_line}` : ""}
            </td>
            <td style={{ padding: "4px 8px" }}>{r.severity ?? "—"}</td>
            <td style={{ padding: "4px 8px", fontFamily: "monospace", fontSize: 11 }}>{r.rule ?? "—"}</td>
            <td style={{ padding: "4px 8px" }}>{r.message ?? "—"}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

type TestCase = { classname?: string; name?: string; status?: string; time_ms?: number | null };
type TestSuite = { name?: string; cases?: TestCase[] };

function TestTree({ findings }: { findings: MetricFinding[] }) {
  const payload = parseDetailPayload(findings, "test-detail") as { suites?: TestSuite[] } | null;
  if (!payload?.suites?.length) return <div style={{ opacity: 0.6 }}>No test detail for the latest run.</div>;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      {payload.suites.map((suite, si) => (
        <div key={si}>
          <div style={{ fontWeight: 600, fontSize: 12 }}>{suite.name ?? "(suite)"}</div>
          <ul style={{ margin: "4px 0 0 16px", padding: 0, fontSize: 12, listStyle: "none" }}>
            {(suite.cases ?? []).map((c, ci) => {
              const failed = c.status === "failed" || c.status === "error";
              return (
                <li key={ci} style={{ color: failed ? "var(--err, #f85149)" : undefined, padding: "1px 0" }}>
                  {failed ? "✗" : "✓"} {c.classname ? `${c.classname}.` : ""}
                  {c.name ?? "(case)"}
                  {c.time_ms != null ? <span style={{ opacity: 0.5 }}> · {c.time_ms}ms</span> : null}
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </div>
  );
}

type CoverageFile = { path?: string; uncoveredChangedLines?: number[] };
type CoveragePayload = { summaryPct?: number; files?: CoverageFile[] };

function CoverageHeat({ findings }: { findings: MetricFinding[] }) {
  const payload = parseDetailPayload(findings, "coverage-detail") as CoveragePayload | null;
  if (!payload?.files?.length) return <div style={{ opacity: 0.6 }}>No coverage detail for the latest run.</div>;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      {payload.summaryPct != null ? (
        <div style={{ fontSize: 12, opacity: 0.7 }}>Summary: {fmt(payload.summaryPct)}%</div>
      ) : null}
      <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
        <thead>
          <tr style={{ textAlign: "left", opacity: 0.6 }}>
            <th style={{ padding: "4px 8px" }}>File</th>
            <th style={{ padding: "4px 8px" }}>Uncovered changed lines</th>
          </tr>
        </thead>
        <tbody>
          {payload.files.map((file, fi) => {
            const lines = file.uncoveredChangedLines ?? [];
            return (
              <tr key={fi} style={{ borderTop: "1px solid var(--border, #2a2a2a)" }}>
                <td style={{ padding: "4px 8px", fontFamily: "monospace", fontSize: 11 }}>{file.path ?? "—"}</td>
                <td
                  style={{
                    padding: "4px 8px",
                    fontFamily: "monospace",
                    fontSize: 11,
                    color: lines.length ? "var(--err, #f85149)" : "var(--ok, #3fb950)",
                  }}
                >
                  {lines.length ? lines.join(", ") : "fully covered"}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function TopSubjects({ samples }: { samples: MetricSample[] }) {
  const top = topSubjects(samples, 10);
  if (top.length === 0) return <div style={{ opacity: 0.6 }}>No subject breakdown.</div>;
  const max = Math.max(...top.map((t) => t.value)) || 1;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
      {top.map((t) => (
        <div key={t.subject} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
          <span style={{ width: 200, fontFamily: "monospace", fontSize: 11, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {t.subject}
          </span>
          <span style={{ flex: 1, background: "var(--border, #2a2a2a)", borderRadius: 2, height: 12 }}>
            <span style={{ display: "block", width: `${(t.value / max) * 100}%`, background: "var(--accent, #58a6ff)", height: 12, borderRadius: 2 }} />
          </span>
          <span style={{ width: 56, textAlign: "right" }}>{fmt(t.value)}</span>
        </div>
      ))}
    </div>
  );
}

export function MetricDetail({
  def,
  onBack,
  effort,
}: {
  def: MetricDefinition;
  onBack: () => void;
  /** When set, show an "In this effort" before→after callout scoped to this
   *  effort window (the task-page metrics-panel drill-in). */
  effort?: { effortId: string; start: string; end: string | null };
}) {
  const [samples, setSamples] = useState<MetricSample[]>([]);
  const [findings, setFindings] = useState<MetricFinding[]>([]);
  const [effortDelta, setEffortDelta] = useState<EffortMetricDelta | null>(null);

  useEffect(() => {
    let cancelled = false;
    void listMetricSamples(def.key, SAMPLE_LIMIT).then(async (rows) => {
      if (cancelled) return;
      setSamples(rows);
      const runId = rows[0]?.run_id ?? null;
      const fs = runId != null ? await listMetricFindings(runId) : [];
      if (!cancelled) setFindings(fs);
    });
    return () => {
      cancelled = true;
    };
  }, [def.key]);

  useEffect(() => {
    if (!effort) {
      setEffortDelta(null);
      return;
    }
    let cancelled = false;
    void listEffortMetricDeltas(effort.effortId).then((rows) => {
      if (!cancelled) setEffortDelta(rows.find((r) => r.key === def.key) ?? null);
    });
    return () => {
      cancelled = true;
    };
  }, [effort?.effortId, def.key, effort]);

  const latest = samples[0] ?? null;
  const delta = deltaVsFirst(samples);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }} data-testid="metric-detail">
      <div style={{ display: "flex", alignItems: "baseline", gap: 12 }}>
        <button onClick={onBack} style={{ fontSize: 12, cursor: "pointer" }}>
          ← Back
        </button>
        <div style={{ fontWeight: 600 }}>{def.title}</div>
        <code style={{ opacity: 0.5, fontSize: 11 }}>{def.key}</code>
        <span style={{ opacity: 0.5, fontSize: 11 }}>{def.kind}</span>
      </div>
      <div style={{ display: "flex", gap: 20, fontSize: 13, flexWrap: "wrap" }}>
        <div>
          <span style={{ opacity: 0.6 }}>Latest </span>
          <strong>
            {latest ? `${fmt(latest.value)}${def.unit ? ` ${def.unit}` : ""}` : "—"}
          </strong>
        </div>
        {delta != null ? (
          <div>
            <span style={{ opacity: 0.6 }}>Δ this window </span>
            <strong>{delta > 0 ? `+${fmt(delta)}` : fmt(delta)}</strong>
          </div>
        ) : null}
        {latest?.branch ? (
          <div>
            <span style={{ opacity: 0.6 }}>Branch </span>
            <code style={{ fontSize: 11 }}>{latest.branch}</code>
          </div>
        ) : null}
        {latest ? (
          <div style={{ opacity: latest.provenance === "observed" ? 0.6 : 1 }}>
            <span style={{ opacity: 0.6 }}>Trust </span>
            <span title={latest.source}>{latest.provenance === "observed" ? "observed" : `⚠ ${latest.provenance}`}</span>
          </div>
        ) : null}
      </div>
      {effort && effortDelta ? (
        <div
          data-testid="metric-detail-effort"
          style={{
            border: "1px solid var(--border-subtle, #2a2a2a)",
            borderRadius: 6,
            padding: "8px 12px",
            display: "flex",
            gap: 12,
            alignItems: "baseline",
            flexWrap: "wrap",
            fontSize: 13,
          }}
        >
          <span style={{ fontWeight: 600 }}>In this effort</span>
          <span style={{ fontFamily: "var(--font-mono)" }}>
            {deltaSummary(effortDelta)}
          </span>
          {effortDelta.changed &&
          effortDelta.delta != null &&
          effortDelta.agg !== "sum" ? (
            <span style={{ color: deltaColor(effortDelta) }}>
              Δ {fmtSigned(effortDelta.delta)}
            </span>
          ) : null}
          {effortDelta.attributed_files != null &&
          effortDelta.attributed_files > 0 ? (
            <span style={{ opacity: 0.6 }}>
              across {effortDelta.attributed_files}{" "}
              {effortDelta.attributed_files === 1 ? "file" : "files"}
            </span>
          ) : null}
        </div>
      ) : null}
      <TrendChart samples={samples} target={def.target} />
      {def.kind === "findings" ? <FindingsTable findings={findings} /> : null}
      {def.kind === "test" ? <TestTree findings={findings} /> : null}
      {def.kind === "coverage" ? <CoverageHeat findings={findings} /> : null}
      {def.kind === "event" ? <TopSubjects samples={samples} /> : null}
    </div>
  );
}
