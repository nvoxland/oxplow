// The per-effort **Metrics** panel on the task page: the metrics whose facts
// were collected during an effort, as compact before→after rows grouped by
// type. Each row drills into the metric's detail page scoped to the effort
// window (further exploration). Data is the family-attributed roll-up from
// `CollectionService::effort_metric_deltas` (see .context/metrics.md) — so the
// numbers stay correct even when efforts overlap.

import React, { useEffect, useState } from "react";

import { type EffortMetricDelta, listEffortMetricDeltas } from "../api.js";
import { subscribeOxplowEvents } from "../tauri-bridge/index.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { metricRef } from "../tabs/pageRefs.js";

/** Group + label a metric delta for the effort panel. Producer categories map
 *  to friendly labels; the code-health gauges (no category) group by language.
 *  Pure — unit-tested. */
export function metricGroup(d: EffortMetricDelta): { order: number; label: string } {
  switch (d.category) {
    case "coverage":
      return { order: 1, label: "Coverage" };
    case "testing":
      return { order: 2, label: "Tests" };
    case "static-quality":
      return { order: 3, label: "Static analysis" };
    case "operational":
      return { order: 4, label: "Activity" };
    case "custom":
      return { order: 5, label: "Custom" };
    default: {
      const lang = d.language
        ? d.language[0].toUpperCase() + d.language.slice(1)
        : null;
      return { order: 0, label: lang ? `${lang} code health` : "Code health" };
    }
  }
}

/** Compact value: integers as-is, else one decimal; ≥10k → `k`. */
export function fmtMetricValue(v: number): string {
  if (Math.abs(v) >= 10_000) return `${(v / 1000).toFixed(1)}k`;
  return Number.isInteger(v) ? String(v) : v.toFixed(1);
}

/** A signed delta (`+3`, `-2`). */
export function fmtSigned(v: number): string {
  const s = fmtMetricValue(Math.abs(v));
  return v < 0 ? `-${s}` : `+${s}`;
}

/** The value-cell text: before→after when the effort moved it, a flow total
 *  for `sum` metrics, else the current value. Units glue for `%`. */
export function deltaSummary(d: EffortMetricDelta): string {
  const unit = d.unit && d.unit !== "count" ? d.unit : "";
  const withUnit = (n: string) =>
    unit === "%" ? `${n}%` : unit ? `${n} ${unit}` : n;
  if (d.agg === "sum") return fmtSigned(d.current);
  if (d.changed && d.baseline != null) {
    return `${withUnit(fmtMetricValue(d.baseline))} → ${withUnit(fmtMetricValue(d.current))}`;
  }
  return withUnit(fmtMetricValue(d.current));
}

/** Color the Δ by whether the move was an improvement (per `direction`). */
export function deltaColor(d: EffortMetricDelta): string {
  if (d.delta == null || d.direction === "neutral") return "var(--text-muted)";
  const improved = d.direction === "lower-better" ? d.delta < 0 : d.delta > 0;
  return improved ? "var(--success, #3fb950)" : "var(--danger, #e5534b)";
}

/**
 * The grouped metrics panel for one effort. Self-hides when the effort touched
 * no tracked metric; live-refreshes on `metricSamplesChanged`.
 */
export function EffortMetricsBlock({
  effortId,
  startedAt,
  endedAt,
}: {
  effortId: string;
  startedAt: string;
  endedAt: string | null;
}) {
  const [deltas, setDeltas] = useState<EffortMetricDelta[]>([]);
  const nav = useOptionalPageNavigation();

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      void listEffortMetricDeltas(effortId).then((rows) => {
        if (!cancelled) setDeltas(rows);
      });
    };
    load();
    const unsub = subscribeOxplowEvents((event) => {
      if (event.kind === "metricSamplesChanged") load();
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [effortId]);

  if (deltas.length === 0) return null;

  // Group, preserving the backend's within-group ordering.
  const groups = new Map<
    string,
    { order: number; label: string; items: EffortMetricDelta[] }
  >();
  for (const d of deltas) {
    const g = metricGroup(d);
    const entry = groups.get(g.label) ?? { ...g, items: [] };
    entry.items.push(d);
    groups.set(g.label, entry);
  }
  const ordered = [...groups.values()].sort((a, b) => a.order - b.order);

  const open = (d: EffortMetricDelta) => {
    if (!nav) return;
    nav.navigate(metricRef(d.key, { effortId, start: startedAt, end: endedAt }), {
      newTab: false,
    });
  };

  return (
    <div
      data-testid={`effort-metrics-${effortId}`}
      style={{ display: "flex", flexDirection: "column", gap: 8 }}
    >
      <h4>Metrics</h4>
      {ordered.map((group) => (
        <div
          key={group.label}
          style={{ display: "flex", flexDirection: "column", gap: 2 }}
        >
          <div
            style={{
              fontSize: "var(--text-xs)",
              color: "var(--text-muted)",
              textTransform: "uppercase",
              letterSpacing: 0.4,
            }}
          >
            {group.label}
          </div>
          {group.items.map((d) => (
            <button
              key={d.key}
              type="button"
              data-testid={`effort-metric-${d.key}`}
              onClick={() => open(d)}
              title={`Open ${d.title} detail`}
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "baseline",
                gap: 10,
                background: "transparent",
                border: "none",
                padding: "2px 0",
                cursor: nav ? "pointer" : "default",
                font: "inherit",
                color: "var(--text-primary)",
                textAlign: "left",
              }}
            >
              <span
                style={{
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {d.title}
              </span>
              <span
                style={{
                  display: "flex",
                  gap: 8,
                  alignItems: "baseline",
                  flexShrink: 0,
                }}
              >
                <span
                  style={{
                    fontFamily: "var(--font-mono)",
                    fontSize: "var(--text-sm)",
                  }}
                >
                  {deltaSummary(d)}
                </span>
                {d.changed && d.delta != null && d.agg !== "sum" ? (
                  <span style={{ fontSize: "var(--text-xs)", color: deltaColor(d) }}>
                    Δ {fmtSigned(d.delta)}
                  </span>
                ) : null}
                {d.attributed_files != null && d.attributed_files > 0 ? (
                  <span
                    style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}
                  >
                    · {d.attributed_files} {d.attributed_files === 1 ? "file" : "files"}
                  </span>
                ) : null}
                {d.crossing ? (
                  <span
                    style={{
                      fontSize: "var(--text-xs)",
                      color:
                        d.crossing === "fail"
                          ? "var(--danger, #e5534b)"
                          : "var(--warning, #d9a300)",
                    }}
                  >
                    ⚠ {d.crossing}
                  </span>
                ) : null}
              </span>
            </button>
          ))}
        </div>
      ))}
    </div>
  );
}
