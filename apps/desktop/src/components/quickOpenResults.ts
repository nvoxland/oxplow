import type { SearchHit } from "../api.js";

/// Body-search hits merged into the quick-open list after the filename
/// matches. File-kind hits whose path already matched by filename are
/// dropped (they'd be duplicate rows); everything else keeps the
/// backend's BM25 order.
export function dedupeSiteHits(
  hits: SearchHit[],
  matchedFilePaths: ReadonlySet<string>,
): SearchHit[] {
  return hits.filter((h) => !(h.kind === "file" && matchedFilePaths.has(h.ref_id)));
}
