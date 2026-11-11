/**
 * Knowledge pages read through the datasets (`.context/knowledge.md`):
 * `v_knowledge_page` for the index, `v_knowledge_body` for a page's
 * markdown, `v_capability_provider` for the enabled stores (oxplow's own
 * is oxwiki). Writes are `knowledge.*` commands (`writeKnowledgePage`).
 */
import { querySql } from "./api.js";
import { unionReads } from "./viz/rerun.js";
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

/** A kind of page a store keeps. */
export interface KnowledgeKind {
  id: string;
  title: string;
  bucket: string;
}

/** An enabled knowledge store. */
export interface KnowledgeStore {
  /** `oxwiki` for oxplow's own. */
  store: string;
  title: string;
  kinds: KnowledgeKind[];
  /** Whether oxplow writes its pages (`features.edit`; one that doesn't say, doesn't). */
  edit: boolean;
}

/** The store of oxplow's own wiki. */
export { OXWIKI } from "./knowledgeStoreIds.js";

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
       FROM v_knowledge_page ORDER BY updated_at DESC`,
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
  const [pages, body] = await Promise.all([
    querySql(`SELECT ${PAGE_COLUMNS} FROM v_knowledge_page WHERE uri = ?1`, [uri], 1),
    querySql("SELECT body FROM v_knowledge_body WHERE uri = ?1", [uri], 1),
  ]);
  const reads = unionReads([pages.reads, body.reads]);
  const summary = pagesFromResult(pages)[0];
  if (!summary) return { page: null, reads };
  return { page: { summary, body: String(body.rows[0]?.[0] ?? "") }, reads };
}

/** What the knowledge surfaces say when no store is enabled. */
export const NO_KNOWLEDGE_STORE = "No knowledge store is enabled — turn oxwiki on in Settings → Capabilities";

function jsonOf<T>(raw: SqlCell | undefined, fallback: T): T {
  try {
    const parsed: unknown = JSON.parse(String(raw ?? ""));
    return parsed && typeof parsed === "object" ? (parsed as T) : fallback;
  } catch {
    return fallback;
  }
}

/** The enabled stores in a `v_capability_provider` read of the knowledge
 *  capability (`provider AS store, title, kinds, features`). */
export function storesFromResult(result: SqlQueryResult): KnowledgeStore[] {
  return result.rows.map((row) => {
    const kinds = jsonOf<unknown>(cell(result, row, "kinds"), []);
    const features = jsonOf<Record<string, unknown>>(cell(result, row, "features"), {});
    return {
      store: String(cell(result, row, "store")),
      title: String(cell(result, row, "title") ?? cell(result, row, "store")),
      kinds: Array.isArray(kinds) ? (kinds as KnowledgeKind[]) : [],
      edit: features.edit === true,
    };
  });
}

/** The enabled knowledge stores, and what the read read. */
export async function readKnowledgeStores(): Promise<{ stores: KnowledgeStore[]; reads: Reads }> {
  const res = await querySql(
    `SELECT provider AS store, title, kinds, features FROM v_capability_provider
      WHERE capability = 'knowledge' AND active = 1 ORDER BY provider`,
    [],
    100,
  );
  return { stores: storesFromResult(res), reads: res.reads };
}
