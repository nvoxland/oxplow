// Formatting for an effort's metric delta (before→after, signed Δ, and
// whether the move improved the metric), used by the metric detail page.
// The effort-review Metrics section is an oxplow-analytics lens now.

import { formatMetricValue } from "./format";

import type { EffortMetricDelta } from "../api.js";

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
