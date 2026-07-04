/// Repaint throttling for the terminal comment overlay.
///
/// The agent pane streams output a line at a time; each write fires
/// `onWriteParsed`. Re-anchoring every comment against a freshly
/// serialized full scrollback on every write (or even every animation
/// frame) is what stalls the main thread — a ~5000-line buffer serializes
/// to ~1MB and each comment scans it. We instead bound repaints to at most
/// one per `REPAINT_MIN_INTERVAL_MS`, with a trailing run so the final
/// state is always correct once the firehose quiets. Comment highlights
/// lagging streamed output by a few hundred ms is imperceptible.

/// Minimum wall-clock gap between comment-overlay repaints while the
/// terminal write firehose is active.
export const REPAINT_MIN_INTERVAL_MS = 250;

export type RepaintPlan = { run: "now" } | { run: "defer"; waitMs: number };

/// Decide how to service a repaint request given when the last repaint ran.
/// Runs immediately once the interval has elapsed; otherwise defers by the
/// remaining time so a trailing repaint captures the latest buffer.
export function planRepaint(lastRunAt: number, now: number, minIntervalMs: number): RepaintPlan {
  const elapsed = now - lastRunAt;
  if (elapsed >= minIntervalMs) return { run: "now" };
  return { run: "defer", waitMs: minIntervalMs - elapsed };
}
