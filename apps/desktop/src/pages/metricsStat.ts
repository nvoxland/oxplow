/**
 * The per-line stat the Metrics rail dropdown selects (tsk115): what
 * number terminates each `title · sparkline · value` row. Every stat is
 * computed over the SAME filtered samples the sparkline plots (newest first,
 * as the page holds them), so the number always describes the visible graph —
 * never a wider window than the line the eye just followed.
 */

export type LineStat = "latest" | "change" | "distance" | "pctTarget" | "mean" | "min" | "max";

export const LINE_STATS: { key: LineStat; label: string }[] = [
  { key: "latest", label: "Latest value" },
  { key: "change", label: "Change" },
  { key: "distance", label: "Distance to target" },
  { key: "pctTarget", label: "Percent of target" },
  { key: "mean", label: "Mean" },
  { key: "min", label: "Min" },
  { key: "max", label: "Max" },
];

export const DEFAULT_LINE_STAT: LineStat = "latest";

/**
 * The stat's value over the plotted samples, or `null` when it isn't
 * computable (no samples; a change needs two points; a target-relative stat
 * with no target) — rendered as "—". `change` is signed newest − oldest across
 * the window. The two target stats (tsk120) read the **latest plotted sample**
 * against the metric's `target`: `distance` = latest − target (signed, in the
 * metric's unit), `pctTarget` = latest / target × 100 (a percent, 100% = on
 * target); both need a target and `pctTarget` needs a non-zero one.
 */
export function lineStatValue(
  samples: ReadonlyArray<{ value: number }>,
  stat: LineStat,
  target?: number | null,
): number | null {
  if (samples.length === 0) return null;
  switch (stat) {
    case "latest":
      return samples[0].value;
    case "change":
      return samples.length < 2 ? null : samples[0].value - samples[samples.length - 1].value;
    case "distance":
      return target == null ? null : samples[0].value - target;
    case "pctTarget":
      return target == null || target === 0 ? null : (samples[0].value / target) * 100;
    case "mean":
      return samples.reduce((acc, s) => acc + s.value, 0) / samples.length;
    case "min":
      return Math.min(...samples.map((s) => s.value));
    case "max":
      return Math.max(...samples.map((s) => s.value));
  }
}
