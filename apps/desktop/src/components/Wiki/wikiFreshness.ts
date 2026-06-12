/// The single freshness derivation for a wiki page. Every widget that
/// talks about a page's freshness (header chip, rail badge, referenced-
/// files footer) must derive from the same `list_wiki_freshness` rows
/// through this function — two widgets computing freshness from
/// different sources is how the header said "3 stale refs" while the
/// rail said "fresh" (the rail used to read vestigial fields that the
/// Rust backend never populates).

import type { WikiRefFreshness } from "../../tauri-bridge/generated/bindings.js";

export type WikiFreshnessLevel = "fresh" | "stale" | "very-stale";

export interface WikiFreshnessSummary {
  totalRefs: number;
  /// Paths of the refs whose latest snapshot is newer than the pin.
  staleRefs: string[];
  freshness: WikiFreshnessLevel;
}

export function summarizeWikiFreshness(rows: WikiRefFreshness[]): WikiFreshnessSummary {
  const staleRefs = rows.filter((r) => r.stale).map((r) => r.path);
  const freshness: WikiFreshnessLevel =
    staleRefs.length === 0 ? "fresh" : staleRefs.length === rows.length ? "very-stale" : "stale";
  return { totalRefs: rows.length, staleRefs, freshness };
}
