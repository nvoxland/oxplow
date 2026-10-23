import type { SeriesPoint } from "../api.js";
import { seriesPoints } from "./metricDetailData.js";

// Pure helpers behind the dashboard tiles. Kept React-free so they are unit-testable without mounting
// anything — same split as `metricDetailData.ts`.

/** The newest sample's value (largest `captured_at`), or `null` when nothing
 *  parses. Order-independent — reuses the same `seriesPoints` sort the trend
 *  chart uses, so the "latest" is consistent with what the chart plots. */
export function latestValue(samples: SeriesPoint[]): number | null {
  const pts = seriesPoints(samples);
  return pts.length ? pts[pts.length - 1]!.v : null;
}

/** Whether a change is good / bad / neutral given the metric's preferred
 *  `direction` (`higher-better` | `lower-better` | `neutral`). A zero delta or
 *  a neutral/unknown direction is `neutral` (no color). Drives the number
 *  tile's delta chip color. */
export function deltaTone(delta: number, direction: string): "good" | "bad" | "neutral" {
  if (delta === 0) return "neutral";
  if (direction === "higher-better") return delta > 0 ? "good" : "bad";
  if (direction === "lower-better") return delta > 0 ? "bad" : "good";
  return "neutral";
}
