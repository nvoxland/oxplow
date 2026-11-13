/**
 * Knowledge pages read through the datasets (`.context/knowledge.md`):
 * `v_oxplow_knowledge_page` for the index and a page's markdown (`body`),
 * `v_model_store` for the stores.
 * Writes are a store's own commands (oxwiki: `oxwiki.page.write`).
 */
import { querySql } from "./api.js";
import type { Reads, SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** One page as the index lists it. */
export interface KnowledgePageSummary {
  /** `oxplow:oxplow/knowledge?id=<id>&store=<store>`. */
  uri: string;
  store: string;
  /** Its id in its store (an oxwiki page's slug). */
  id: string;
  kind: string;
  title: string;
  excerpt: string;
  updated_at: string;
  /** Its file refs whose file changed since they were pinned. */
  stale_ref_count: number;
  /** Every ref it links to (`file:src/a.rs`, a page's URI). */
  outbound_uris: string[];
}

/** A knowledge store: a store of the knowledge model (`v_model_store`). */
export interface KnowledgeStore {
  store: string;
  /** The extension that keeps it. */
  extension: string;
  /** How its page ids look in text, a regex matched whole; null when it doesn't say. */
  idPattern: string | null;
}

// Every read returns what it read (`reads`); a consumer re-runs it through
// `useRerunOnChange` (or `readsChanged`) when one of those datasets changes.

const cell = (result: SqlQueryResult, row: SqlCell[], name: string) => row[result.columns.indexOf(name)];

export function pagesFromResult(result: SqlQueryResult): KnowledgePageSummary[] {
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
      store: String(cell(result, row, "store")),
      id: String(cell(result, row, "id")),
      kind: String(cell(result, row, "kind") ?? ""),
      title: String(cell(result, row, "title") ?? ""),
      excerpt: String(cell(result, row, "excerpt") ?? ""),
      updated_at: String(cell(result, row, "updated_at") ?? ""),
      stale_ref_count: Number(cell(result, row, "stale_ref_count") ?? 0),
      outbound_uris: outbound,
    };
  });
}

const PAGE_COLUMNS = "uri, store, id, kind, title, excerpt, updated_at, stale_ref_count, outbound_uris";

/** Every page of an enabled store, most recently updated first. */
export async function readKnowledgePages(): Promise<{ pages: KnowledgePageSummary[]; reads: Reads }> {
  const res = await querySql(
    `SELECT ${PAGE_COLUMNS}
       FROM v_oxplow_knowledge_page ORDER BY updated_at DESC`,
    [],
    100_000,
  );
  return { pages: pagesFromResult(res), reads: res.reads };
}

/** One page's summary and body, by its URI (null when there's no such page),
 *  and what the read read. */
export async function readKnowledgePage(
  uri: string,
): Promise<{ page: { summary: KnowledgePageSummary; body: string } | null; reads: Reads }> {
  const pages = await querySql(`SELECT ${PAGE_COLUMNS}, body FROM v_oxplow_knowledge_page WHERE uri = ?1`, [uri], 1);
  const summary = pagesFromResult(pages)[0];
  if (!summary) return { page: null, reads: pages.reads };
  return { page: { summary, body: String(cell(pages, pages.rows[0]!, "body") ?? "") }, reads: pages.reads };
}

/** What the knowledge surfaces say when no store is enabled. */
export const NO_KNOWLEDGE_STORE = "No knowledge store is on — turn on an extension that keeps pages, such as oxwiki";

/** The stores in a `v_model_store` read of the knowledge model
 *  (`store, extension, id_pattern`). */
export function storesFromResult(result: SqlQueryResult): KnowledgeStore[] {
  return result.rows.map((row) => ({
    store: String(cell(result, row, "store")),
    extension: String(cell(result, row, "extension") ?? ""),
    idPattern: cell(result, row, "id_pattern") == null ? null : String(cell(result, row, "id_pattern")),
  }));
}

/** The knowledge stores, and what the read read. */
export async function readKnowledgeStores(): Promise<{ stores: KnowledgeStore[]; reads: Reads }> {
  const res = await querySql(
    "SELECT store, extension, id_pattern FROM v_model_store WHERE model = 'oxplow/knowledge_record' ORDER BY store",
    [],
    100,
  );
  return { stores: storesFromResult(res), reads: res.reads };
}
