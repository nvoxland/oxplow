/**
 * Formatting helpers shared across pages and panels.
 *
 * Kept lib-style (no React, no DOM) so any module can pull them in
 * without dragging UI dependencies. Live here rather than in api.ts
 * to keep the api module focused on IPC wrappers.
 */

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

/**
 * Short date+time label used in dashboard rows: "May 13, 14:32".
 * Falls back to the raw string if `Date` can't parse it.
 */
export function formatShortDateTime(input: string): string {
  try {
    return new Date(input).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return input;
  }
}

/**
 * Full date+time label used on detail-page headers: locale long form.
 * Falls back to the raw string if `Date` can't parse it.
 */
export function formatFullDateTime(input: string): string {
  try {
    return new Date(input).toLocaleString();
  } catch {
    return input;
  }
}

/**
 * Time-only label (locale form, no date). Used to collapse a date range
 * whose endpoints fall on the same day. Falls back to the raw string if
 * `Date` can't parse it.
 */
export function formatTimeOnly(input: string): string {
  const d = new Date(input);
  if (Number.isNaN(d.getTime())) return input;
  return d.toLocaleTimeString();
}

/**
 * Date-only label (locale form, no time). Used for the diff range's
 * date line. Falls back to the raw string if `Date` can't parse it.
 */
export function formatDateOnly(input: string): string {
  const d = new Date(input);
  if (Number.isNaN(d.getTime())) return input;
  return d.toLocaleDateString();
}

// --- Metric value formatting (tsk114) --------------------------------------
//
// ONE formatter for every metric value the UI shows, built on
// `Intl.NumberFormat` with the OS locale (`undefined` = system): grouping
// separators, decimal comma vs point, and compact notation all follow the
// user's locale for free. Deliberately NO user-facing locale setting yet —
// this module is the single seam, so a future override is a one-line change
// here instead of a call-site hunt. Per-metric presentation rides the spec's
// existing `unit` (`%`, `ms`, `count`, `tokens`, `lines`, …), never a global
// settings panel.

const exactFmt = new Intl.NumberFormat(undefined, { maximumFractionDigits: 4 });
const smallFmt = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
const oneDecimalFmt = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });
const compactFmt = new Intl.NumberFormat(undefined, {
  notation: "compact",
  maximumFractionDigits: 1,
});

/**
 * Above this, plain numbers render compact ("240.4K"). A surface showing a
 * compacted value should pair it with [`formatMetricValueExact`] in a hover
 * `title` so the precise number is one hover away.
 */
const COMPACT_AT = 10_000;

/**
 * A metric value for display, unit-aware:
 * - `%` → one decimal + `%` (spec values are already ×100 scaled);
 * - `ms` → humanized duration (`850 ms`, `1.2 s`, `3.4 m`);
 * - everything else → locale-grouped, compact at ≥10k, with the unit
 *   appended when present (`1,234 lines`, `240.4K count`… units that read
 *   awkwardly appended are the spec author's naming choice, not ours).
 */
export function formatMetricValue(value: number, unit?: string | null): string {
  if (!Number.isFinite(value)) return String(value);
  if (unit === "%") return `${oneDecimalFmt.format(value)}%`;
  if (unit === "ms") return formatDurationMs(value);
  const body =
    Math.abs(value) >= COMPACT_AT ? compactFmt.format(value) : smallFmt.format(value);
  return unit ? `${body} ${unit}` : body;
}

/** Humanize a millisecond quantity: `850 ms`, `1.2 s`, `3.4 m`, `1.1 h`. */
export function formatDurationMs(ms: number): string {
  if (!Number.isFinite(ms)) return String(ms);
  const abs = Math.abs(ms);
  if (abs < 1_000) return `${oneDecimalFmt.format(ms)} ms`;
  if (abs < 60_000) return `${oneDecimalFmt.format(ms / 1_000)} s`;
  if (abs < 3_600_000) return `${oneDecimalFmt.format(ms / 60_000)} m`;
  return `${oneDecimalFmt.format(ms / 3_600_000)} h`;
}

/**
 * The exact (never compacted) locale-grouped form — the hover `title`
 * companion for compacted displays.
 */
export function formatMetricValueExact(value: number, unit?: string | null): string {
  if (!Number.isFinite(value)) return String(value);
  const body = exactFmt.format(value);
  return unit ? `${body} ${unit}` : body;
}

/**
 * True when two parseable timestamps fall on the same local calendar
 * day. False if either can't be parsed.
 */
export function isSameCalendarDay(a: string, b: string): boolean {
  const da = new Date(a);
  const db = new Date(b);
  if (Number.isNaN(da.getTime()) || Number.isNaN(db.getTime())) return false;
  return (
    da.getFullYear() === db.getFullYear() &&
    da.getMonth() === db.getMonth() &&
    da.getDate() === db.getDate()
  );
}
