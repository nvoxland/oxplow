import type { MetricFinding, MetricSample } from "../api.js";

// Pure helpers behind the per-kind Metric detail view (tsk232). Kept out of the
// component so they're unit-testable without a DOM — same split as
// `buildExplorerSeries` in MetricsExplorer.

export type SeriesPoint = { t: number; v: number };

/** Samples (any order) → time-ascending `{t,v}` points, dropping unparseable
 *  timestamps. The shared input to every kind's trend chart. */
export function seriesPoints(samples: MetricSample[]): SeriesPoint[] {
  return samples
    .map((s) => ({ t: Date.parse(String(s.captured_at)), v: s.value }))
    .filter((p) => !Number.isNaN(p.t))
    .sort((a, b) => a.t - b.t);
}

/** Latest value minus the earliest (the effort/window delta). `null` when there
 *  are fewer than two points. */
export function deltaVsFirst(samples: MetricSample[]): number | null {
  const pts = seriesPoints(samples);
  if (pts.length < 2) return null;
  return pts[pts.length - 1]!.v - pts[0]!.v;
}

/** Top-N subjects by summed value — the `event`/`findings` "where is it
 *  concentrated" breakdown. Groups by `subject_ref` (falling back to
 *  `subject_kind`); samples with no subject are bucketed under "—". */
export function topSubjects(
  samples: MetricSample[],
  n: number,
): { subject: string; value: number }[] {
  const sums = new Map<string, number>();
  for (const s of samples) {
    const key = s.subject_ref ?? s.subject_kind ?? "—";
    sums.set(key, (sums.get(key) ?? 0) + s.value);
  }
  return [...sums.entries()]
    .map(([subject, value]) => ({ subject, value }))
    .sort((a, b) => b.value - a.value)
    .slice(0, n);
}

/** The real findings rows (lint hits, complexity, …) for the findings-kind
 *  table — excludes the verbatim `*-detail` payload findings the producers
 *  attach for the effort panel. */
export function findingRows(findings: MetricFinding[]): MetricFinding[] {
  return findings.filter((f) => !f.kind.endsWith("-detail"));
}

/** Parse a `*-detail` finding's `extra_json` payload (the test suite/case tree
 *  or coverage file/line map kept verbatim by the producers). `null` when
 *  absent or unparseable. */
export function parseDetailPayload(
  findings: MetricFinding[],
  detailKind: string,
): unknown | null {
  const f = findings.find((x) => x.kind === detailKind);
  if (!f?.extra_json) return null;
  try {
    return JSON.parse(f.extra_json) as unknown;
  } catch {
    return null;
  }
}
