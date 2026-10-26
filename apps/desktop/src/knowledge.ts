/**
 * Knowledge pages (the wiki) read through the models (P6.E2,
 * `.context/knowledge.md`): `v_knowledge_page` for the index,
 * `v_knowledge_body` for a page's markdown.
 * Writes are `knowledge.*` commands (`writeWikiPage`).
 */
import { querySql } from "./api.js";
import { wikiUri } from "./uri/uri.js";
import { unionReads } from "./viz/rerun.js";
import type { CapabilityProvider } from "./workItems.js";
import type { Reads, SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** One page as the index lists it. */
export interface WikiPageSummary {
  /** `wiki:<slug>`. */
  uri: string;
  slug: string;
  title: string;
  excerpt: string;
  updated_at: string;
  /** Its file refs whose file changed since they were pinned. */
  stale_ref_count: number;
  /** Every ref it links to (`file:src/a.rs`, `wiki:other`). */
  outbound_uris: string[];
}

// Every read returns what it read (`reads`); a consumer re-runs it through
// `useRerunOnChange` (or `readsChanged`) when one of those models changes.

const cell = (result: SqlQueryResult, row: SqlCell[], name: string) => row[result.columns.indexOf(name)];

export function pagesFromResult(result: SqlQueryResult): WikiPageSummary[] {
  return result.rows.map((row) => {
    let outbound: string[] = [];
    try {
      const parsed: unknown = JSON.parse(String(cell(result, row, "outbound_uris") ?? "[]"));
      if (Array.isArray(parsed)) outbound = parsed.map(String);
    } catch {
      outbound = [];
    }
    return {
      uri: String(cell(result, row, "uri")),
      slug: String(cell(result, row, "slug")),
      title: String(cell(result, row, "title") ?? ""),
      excerpt: String(cell(result, row, "excerpt") ?? ""),
      updated_at: String(cell(result, row, "updated_at") ?? ""),
      stale_ref_count: Number(cell(result, row, "stale_ref_count") ?? 0),
      outbound_uris: outbound,
    };
  });
}

/** Every page, most recently updated first. */
export async function readWikiPages(): Promise<{ pages: WikiPageSummary[]; reads: Reads }> {
  const res = await querySql(
    `SELECT uri, slug, title, excerpt, updated_at, stale_ref_count, outbound_uris
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
      `SELECT uri, slug, title, excerpt, updated_at, stale_ref_count, outbound_uris
         FROM v_knowledge_page WHERE slug = ?1`,
      [slug],
      1,
    ),
    querySql("SELECT body FROM v_knowledge_body WHERE uri = ?1", [wikiUri(slug)], 1),
  ]);
  const reads = unionReads([pages.reads, body.reads]);
  const summary = pagesFromResult(pages)[0];
  if (!summary) return { page: null, reads };
  return { page: { summary, body: String(body.rows[0]?.[0] ?? "") }, reads };
}

/** What the wiki's surfaces say when knowledge's active row is `none`. */
export const NO_KNOWLEDGE_STORE = "No knowledge store — choose one in Settings → Capabilities";

/** Whether knowledge's active row is `none` (`providers` being its rows of
 *  `v_capability_provider`); nothing listed yet is not none. */
export function knowledgeIsNone(providers: CapabilityProvider[]): boolean {
  return providers.some((p) => p.capability === "knowledge" && p.active && p.provider === "none");
}
