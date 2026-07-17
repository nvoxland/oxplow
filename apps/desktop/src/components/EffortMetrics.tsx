// The per-effort **Metrics** panel on the task page: the metrics whose facts
// were collected during an effort, as compact before→after rows grouped by
// type. Each row drills into the metric's detail page scoped to the effort
// window (further exploration). Data is the family-attributed roll-up from
// `CollectionService::effort_metric_deltas` (see .context/metrics.md) — so the
// numbers stay correct even when efforts overlap.

import React, { useEffect, useState } from "react";
import { formatMetricValue } from "./format";

import { type EffortMetricDelta, listEffortMetricDeltas } from "../api.js";
import { subscribeOxplowEvents } from "../tauri-bridge/index.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { metricRef } from "../tabs/pageRefs.js";

/** Whether a metric already has a richer, dedicated panel in
 *  `EffortObservationsBlock` (Coverage, Tests run, Static analysis, Token usage,
 *  Agent nudges) — hidden here so the generic Metrics list doesn't repeat it.
 *  Leaves this block to its unique value: code-health gauges + operational
 *  metrics with no panel (cycle time, efforts). Pure — unit-tested. */
export function hasDedicatedPanel(d: EffortMetricDelta): boolean {
  if (
    d.category === "coverage" ||
    d.category === "testing" ||
    d.category === "static-quality"
  ) {
    return true;
  }
  return d.key.startsWith("agent.tokens.") || d.key.startsWith("agent.nudges.");
}

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

/** Compact metric value — delegates to the shared locale-aware formatter
 * (tsk114) so effort chips read like every other metric surface. */
export function fmtMetricValue(v: number): string {
  return formatMetricValue(v);
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
 * The grouped metrics panel for one effort. By default self-hides when the
 * effort touched no tracked metric; pass `showWhenEmpty` to instead render an
 * explicit "No metrics collected" state (used on the diff view so the Metrics
 * section is always present for an effort). Live-refreshes on
 * `metricSamplesChanged`.
 */
export function EffortMetricsBlock({
  effortId,
  startedAt,
  endedAt,
  showWhenEmpty = false,
}: {
  effortId: string;
  startedAt: string;
  endedAt: string | null;
  showWhenEmpty?: boolean;
}) {
  const [deltas, setDeltas] = useState<EffortMetricDelta[]>([]);
  const nav = useOptionalPageNavigation();

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const load = () => {
      void listEffortMetricDeltas(effortId)
        .then((rows) => {
          if (!cancelled) setDeltas(rows);
        })
        // An IPC failure (or no backend, e.g. in tests) is the same end-state
        // as "no metric moved" — leave the deltas empty.
        .catch(() => {
          if (!cancelled) setDeltas([]);
        });
    };
    load();
    // The delta computation is expensive backend work, and the OTLP token
    // ingest emits metricSamplesChanged every ~10s while an agent runs
    // (tsk75 — the un-debounced reload loop saturated the daemon). An effort
    // closed a while ago is frozen: its window can only gain late captures
    // right around the close (on-effort-complete gauges, amend), so stop
    // listening entirely once it's cold. Open/recent efforts coalesce bursts
    // with a trailing debounce.
    const closedLongAgo =
      endedAt !== null && Date.now() - new Date(endedAt).getTime() > 10 * 60_000;
    if (closedLongAgo) {
      return () => {
        cancelled = true;
      };
    }
    const unsub = subscribeOxplowEvents((event) => {
      if (event.kind !== "metricSamplesChanged") return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(load, 2_500);
    });
    return () => {
      cancelled = true;
      unsub();
      if (timer) clearTimeout(timer);
    };
  }, [effortId, endedAt]);

  // Drop metrics that already have a dedicated panel above (tests, coverage,
  // analysis, tokens, nudges) — don't repeat them in the generic list.
  const shown = deltas.filter((d) => !hasDedicatedPanel(d));
  if (shown.length === 0) {
    if (!showWhenEmpty) return null;
    return (
      <div
        data-testid={`effort-metrics-empty-${effortId}`}
        style={{ display: "flex", flexDirection: "column", gap: 8 }}
      >
        <h2 className="task-activity-heading">Metrics</h2>
        <span style={{ fontSize: "var(--text-sm)", color: "var(--text-muted)", fontStyle: "italic" }}>
          No metrics collected for this effort.
        </span>
      </div>
    );
  }

  // Group, preserving the backend's within-group ordering.
  const groups = new Map<
    string,
    { order: number; label: string; items: EffortMetricDelta[] }
  >();
  for (const d of shown) {
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
      <h2 className="task-activity-heading">Metrics</h2>
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
