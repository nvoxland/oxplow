/**
 * The per-line stat the Recorded Metrics rail dropdown selects (tsk115): what
 * number terminates each `title · sparkline · value` row. Every stat is
 * computed over the SAME filtered samples the sparkline plots (newest first,
 * as the page holds them), so the number always describes the visible graph —
 * never a wider window than the line the eye just followed.
 */

export type LineStat = "latest" | "change" | "mean" | "min" | "max";

export const LINE_STATS: { key: LineStat; label: string }[] = [
  { key: "latest", label: "Latest value" },
  { key: "change", label: "Change" },
  { key: "mean", label: "Mean" },
  { key: "min", label: "Min" },
  { key: "max", label: "Max" },
];

export const DEFAULT_LINE_STAT: LineStat = "latest";

/**
 * The stat's value over the plotted samples, or `null` when it isn't
 * computable (no samples; a change needs two points) — rendered as "—".
 * `change` is signed newest − oldest across the window.
 */
export function lineStatValue(
  samples: ReadonlyArray<{ value: number }>,
  stat: LineStat,
): number | null {
  if (samples.length === 0) return null;
  switch (stat) {
    case "latest":
      return samples[0].value;
    case "change":
      return samples.length < 2 ? null : samples[0].value - samples[samples.length - 1].value;
    case "mean":
      return samples.reduce((acc, s) => acc + s.value, 0) / samples.length;
    case "min":
      return Math.min(...samples.map((s) => s.value));
    case "max":
      return Math.max(...samples.map((s) => s.value));
  }
}
