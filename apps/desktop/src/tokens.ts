/**
 * Token-count humanizing for the token-usage UI (tsk104).
 *
 * Tokens-only for now — no cost. Compact forms: `1234 → 1.2K`,
 * `1_200_000 → 1.2M`. Exact small counts (< 1000) render verbatim.
 */

import type { TokenUsageTotals } from "./tauri-bridge/index.js";

function trim1(n: number): string {
  // One decimal, dropping a trailing `.0`.
  return n.toFixed(1).replace(/\.0$/, "");
}

/** Humanize a token count: `936`, `1.2K`, `12K`, `1.2M`. */
export function formatTokens(n: number): string {
  const sign = n < 0 ? "-" : "";
  const v = Math.abs(n);
  if (v < 1000) return `${n}`;
  if (v < 1_000_000) return `${sign}${trim1(v / 1000)}K`;
  return `${sign}${trim1(v / 1_000_000)}M`;
}

/** One-line summary of a totals row, e.g. `1.2M tokens · 8 turns`. */
export function tokenTotalsSummary(t: TokenUsageTotals): string {
  const turns = `${t.turns} turn${t.turns === 1 ? "" : "s"}`;
  return `${formatTokens(t.total_tokens)} tokens · ${turns}`;
}
