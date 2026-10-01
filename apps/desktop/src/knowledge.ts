/**
 * Knowledge pages (the wiki) read through the models (P6.E2,
 * `.context/knowledge.md`): `v_knowledge_page` for the index,
 * `v_knowledge_body` for a page's markdown, `v_knowledge_ref` for its
 * freshness, and the site `search` (`kinds: ["wiki"]`) for finding one.
 * Writes are `knowledge.*` commands (`writeWikiPage`).
 */
import { querySql, searchSite } from "./api.js";
import { unionReads } from "./lens/lensRerun.js";
import type { Reads, SearchHit, SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** One file ref of a page and whether its file drifted since it was
 *  pinned (`v_knowledge_ref`). */
export interface WikiRefFreshness {
  path: string;
  /** The snapshot it was pinned to (written or verified against). */
  pinned_snapshot_id: number | null;
  /** The VCS revision nearest the pin, and whether the pin is exactly it. */
  pinned_vcs_rev: string | null;
  pinned_vcs_rev_exact: boolean;
  /** The file's latest primary-stream snapshot. */
  latest_snapshot_id: number | null;
  stale: boolean;
}

/** One page as the index lists it. */
export interface WikiPageSummary {
  /** `wiki:<slug>`. */
  ref: string;
  slug: string;
  title: string;
  excerpt: string;
  updated_at: string;
  /** Its file refs whose file changed since they were pinned. */
  stale_ref_count: number;
  /** Every ref it links to (`file:src/a.rs`, `wiki:other`). */
  outbound_refs: string[];
}

/** A search hit on a page. */
export interface WikiPageSearchHit {
  slug: string;
  title: string;
  snippet: string;
}

// Every read returns what it read (`reads`); a consumer re-runs it through
// `useRerunOnChange` (or `readsChanged`) when one of those models changes.

const cell = (result: SqlQueryResult, row: SqlCell[], name: string) => row[result.columns.indexOf(name)];

export function pagesFromResult(result: SqlQueryResult): WikiPageSummary[] {
  return result.rows.map((row) => {
    let outbound: string[] = [];
    try {
      const parsed: unknown = JSON.parse(String(cell(result, row, "outbound_refs") ?? "[]"));
      if (Array.isArray(parsed)) outbound = parsed.map(String);
    } catch {
      outbound = [];
    }
    return {
      ref: String(cell(result, row, "ref")),
      slug: String(cell(result, row, "slug")),
      title: String(cell(result, row, "title") ?? ""),
      excerpt: String(cell(result, row, "excerpt") ?? ""),
      updated_at: String(cell(result, row, "updated_at") ?? ""),
      stale_ref_count: Number(cell(result, row, "stale_ref_count") ?? 0),
      outbound_refs: outbound,
    };
  });
}

/** Every page, most recently updated first. */
export async function readWikiPages(): Promise<{ pages: WikiPageSummary[]; reads: Reads }> {
  const res = await querySql(
    `SELECT ref, slug, title, excerpt, updated_at, stale_ref_count, outbound_refs
       FROM v_knowledge_page ORDER BY updated_at DESC`,
    [],
    100_000,
  );
  return { pages: pagesFromResult(res), reads: res.reads };
}

/** One page's summary and body (null when there's no such page), and
 *  what the read read. */
export async function readWikiPage(
  slug: string,
): Promise<{ page: { summary: WikiPageSummary; body: string } | null; reads: Reads }> {
  const [pages, body] = await Promise.all([
    querySql(
      `SELECT ref, slug, title, excerpt, updated_at, stale_ref_count, outbound_refs
         FROM v_knowledge_page WHERE slug = ?1`,
      [slug],
      1,
    ),
    querySql("SELECT body FROM v_knowledge_body WHERE ref = ?1", [`wiki:${slug}`], 1),
  ]);
  const reads = unionReads([pages.reads, body.reads]);
  const summary = pagesFromResult(pages)[0];
  if (!summary) return { page: null, reads };
  return { page: { summary, body: String(body.rows[0]?.[0] ?? "") }, reads };
}

export function freshnessFromResult(result: SqlQueryResult): WikiRefFreshness[] {
  const num = (v: SqlCell | undefined) => (v === null || v === undefined ? null : Number(v));
  return result.rows.map((row) => ({
    path: String(cell(result, row, "path")),
    pinned_snapshot_id: num(cell(result, row, "pinned_snapshot_id")),
    pinned_vcs_rev: cell(result, row, "pinned_vcs_rev") === null ? null : String(cell(result, row, "pinned_vcs_rev")),
    pinned_vcs_rev_exact: Number(cell(result, row, "pinned_vcs_rev_exact") ?? 0) === 1,
    latest_snapshot_id: num(cell(result, row, "latest_snapshot_id")),
    stale: Number(cell(result, row, "stale") ?? 0) === 1,
  }));
}

/** A page's file refs and whether each has drifted since it was pinned. */
export async function readWikiFreshness(slug: string): Promise<{ rows: WikiRefFreshness[]; reads: Reads }> {
  const res = await querySql(
    `SELECT path, pinned_snapshot_id, pinned_vcs_rev, pinned_vcs_rev_exact, latest_snapshot_id, stale
       FROM v_knowledge_ref WHERE page = ?1 ORDER BY path`,
    [`wiki:${slug}`],
    10_000,
  );
  return { rows: freshnessFromResult(res), reads: res.reads };
}

export function searchHitsOf(hits: SearchHit[]): WikiPageSearchHit[] {
  return hits.filter((h) => h.kind === "wiki").map((h) => ({ slug: h.ref_id, title: h.title, snippet: h.snippet }));
}

/** Pages whose title or body matches `query` (the site search). */
export async function searchWikiPages(query: string, limit = 30): Promise<WikiPageSearchHit[]> {
  return searchHitsOf(await searchSite(query, null, ["wiki"], limit));
}
