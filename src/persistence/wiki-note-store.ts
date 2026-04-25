import type { Logger } from "../core/logger.js";
import { createId } from "../core/ids.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";

export interface NoteRefSnapshot {
  path: string;
  blobSha: string | null;
  mtimeMs: number | null;
}

export interface WikiNote {
  id: string;
  slug: string;
  title: string;
  captured_head_sha: string | null;
  captured_refs: NoteRefSnapshot[];
  created_at: string;
  updated_at: string;
}

export interface WikiNoteChange {
  kind: "upserted" | "deleted";
  slug: string | null;
}

export interface UpsertInput {
  slug: string;
  title: string;
  body: string;
  capturedHeadSha: string | null;
  capturedRefs: NoteRefSnapshot[];
}

export interface BodySearchResult {
  slug: string;
  title: string;
  /** ~200 char window centered on the first match, with newlines collapsed. */
  snippet: string;
  updated_at: string;
}

export class WikiNoteStore {
  private readonly stateDb;
  private readonly emitter: StoreEmitter<WikiNoteChange>;

  constructor(projectDir: string, logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.emitter = new StoreEmitter("wiki note", logger);
  }

  subscribe(listener: (change: WikiNoteChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  list(): WikiNote[] {
    return this.stateDb
      .all<Record<string, unknown>>(`SELECT * FROM wiki_note ORDER BY updated_at DESC, id`)
      .map(toWikiNote);
  }

  getBySlug(slug: string): WikiNote | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT * FROM wiki_note WHERE slug = ?`,
      slug,
    );
    return row ? toWikiNote(row) : null;
  }

  searchByTitle(query: string): WikiNote[] {
    const like = `%${query.toLowerCase()}%`;
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM wiki_note WHERE lower(title) LIKE ? ORDER BY updated_at DESC, id`,
        like,
      )
      .map(toWikiNote);
  }

  upsert(input: UpsertInput): WikiNote {
    const now = new Date().toISOString();
    const refsJson = JSON.stringify(input.capturedRefs);
    const existing = this.getBySlug(input.slug);
    if (existing) {
      this.stateDb.run(
        `UPDATE wiki_note
         SET title = ?, body = ?, captured_head_sha = ?, captured_refs_json = ?, updated_at = ?
         WHERE id = ?`,
        input.title,
        input.body,
        input.capturedHeadSha,
        refsJson,
        now,
        existing.id,
      );
    } else {
      const id = createId("wn");
      this.stateDb.run(
        `INSERT INTO wiki_note (id, slug, title, body, captured_head_sha, captured_refs_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
        id,
        input.slug,
        input.title,
        input.body,
        input.capturedHeadSha,
        refsJson,
        now,
        now,
      );
    }
    const saved = this.getBySlug(input.slug);
    if (!saved) throw new Error("wiki note not persisted");
    this.emitter.emit({ kind: "upserted", slug: input.slug });
    return saved;
  }

  deleteBySlug(slug: string): void {
    const existing = this.getBySlug(slug);
    if (!existing) return;
    this.stateDb.run(`DELETE FROM wiki_note WHERE id = ?`, existing.id);
    this.emitter.emit({ kind: "deleted", slug });
  }

  /**
   * Full-text search over note titles + bodies via the `wiki_note_fts`
   * FTS5 virtual table (migration v39). Returns slug, title, updated_at,
   * and a snippet centered on the match (with `<mark>` highlights).
   * Used by the `oxplow__search_note_bodies` MCP tool and by the
   * Notes-pane search input.
   *
   * Falls back to a substring LIKE on title+body when the user query
   * isn't a valid FTS5 expression (raw punctuation, unbalanced quotes,
   * etc.) — better to return loose results than to throw a SQL error
   * into the UI.
   */
  searchBodies(query: string, limit = 20): BodySearchResult[] {
    const trimmed = query.trim();
    if (trimmed.length === 0) return [];
    const ftsQuery = toFtsQuery(trimmed);
    if (ftsQuery !== null) {
      try {
        const rows = this.stateDb.all<Record<string, unknown>>(
          `SELECT n.slug AS slug, n.title AS title, n.updated_at AS updated_at,
                  snippet(wiki_note_fts, 2, '<mark>', '</mark>', '…', 12) AS snippet
             FROM wiki_note_fts f
             JOIN wiki_note n ON n.rowid = f.rowid
            WHERE wiki_note_fts MATCH ?
            ORDER BY rank
            LIMIT ?`,
          ftsQuery,
          limit,
        );
        return rows.map((row) => ({
          slug: String(row.slug),
          title: String(row.title),
          updated_at: String(row.updated_at),
          snippet: String(row.snippet ?? ""),
        }));
      } catch {
        // fall through to LIKE fallback
      }
    }
    const like = `%${trimmed.toLowerCase()}%`;
    const rows = this.stateDb.all<Record<string, unknown>>(
      `SELECT slug, title, body, updated_at
       FROM wiki_note
       WHERE lower(body) LIKE ? OR lower(title) LIKE ?
       ORDER BY updated_at DESC, id
       LIMIT ?`,
      like,
      like,
      limit,
    );
    return rows.map((row) => ({
      slug: String(row.slug),
      title: String(row.title),
      updated_at: String(row.updated_at),
      snippet: makeSnippet(String(row.body ?? ""), trimmed),
    }));
  }

  /**
   * Find notes whose `captured_refs` include the given workspace-relative
   * path. Used by the `oxplow__find_notes_for_file` MCP tool to surface
   * existing notes that already reference a file the agent is exploring.
   * Corpus is small (dozens-to-hundreds of notes), so a full scan + JS
   * filter is fine — no need for a junction table.
   */
  findByRefPath(path: string): WikiNote[] {
    const all = this.list();
    return all.filter((note) =>
      note.captured_refs.some((ref) => ref.path === path),
    );
  }
}

/**
 * Convert a user-typed query into an FTS5 MATCH expression. We tokenize
 * on whitespace, drop FTS5-meaningful characters from each token, quote
 * the result, and AND them together with `*` suffix for prefix-match.
 * Returns null if no usable token survives so the caller falls back to
 * the plain LIKE path.
 */
function toFtsQuery(input: string): string | null {
  const tokens: string[] = [];
  for (const raw of input.split(/\s+/)) {
    const cleaned = raw.replace(/["()*:^~]/g, "").trim();
    if (cleaned.length === 0) continue;
    tokens.push(`"${cleaned}"*`);
  }
  return tokens.length === 0 ? null : tokens.join(" AND ");
}

function makeSnippet(body: string, query: string): string {
  if (body.length === 0) return "";
  const lower = body.toLowerCase();
  const idx = lower.indexOf(query.toLowerCase());
  const window = 100;
  const start = idx < 0 ? 0 : Math.max(0, idx - window);
  const end = idx < 0 ? Math.min(body.length, 200) : Math.min(body.length, idx + query.length + window);
  const slice = body.slice(start, end).replace(/\s+/g, " ").trim();
  const prefix = start > 0 ? "…" : "";
  const suffix = end < body.length ? "…" : "";
  return `${prefix}${slice}${suffix}`;
}

function toWikiNote(row: Record<string, unknown>): WikiNote {
  const refsRaw = String(row.captured_refs_json ?? "[]");
  let refs: NoteRefSnapshot[] = [];
  try {
    const parsed = JSON.parse(refsRaw);
    if (Array.isArray(parsed)) {
      refs = parsed
        .filter((r): r is Record<string, unknown> => typeof r === "object" && r !== null)
        .map((r) => ({
          path: String(r.path ?? ""),
          blobSha: r.blobSha == null ? null : String(r.blobSha),
          mtimeMs: r.mtimeMs == null ? null : Number(r.mtimeMs),
        }))
        .filter((r) => r.path !== "");
    }
  } catch {
    refs = [];
  }
  return {
    id: String(row.id),
    slug: String(row.slug),
    title: String(row.title),
    captured_head_sha: row.captured_head_sha == null ? null : String(row.captured_head_sha),
    captured_refs: refs,
    created_at: String(row.created_at),
    updated_at: String(row.updated_at),
  };
}

export type FreshnessStatus = "fresh" | "stale" | "very-stale";

export interface FreshnessReport {
  status: FreshnessStatus;
  headAdvanced: boolean;
  changedRefs: string[];
  deletedRefs: string[];
  totalRefs: number;
}

/**
 * Pure freshness computation. Caller supplies the current HEAD sha and a
 * resolver that returns each referenced path's current blob sha (or null
 * when the file no longer exists). The freshness indicator is intentionally
 * loose — it's a nudge, not a proof.
 */
export function computeFreshness(
  note: { capturedHeadSha: string | null; capturedRefs: NoteRefSnapshot[] },
  currentHeadSha: string | null,
  resolveBlobSha: (path: string) => string | null,
): FreshnessReport {
  const headAdvanced =
    note.capturedHeadSha != null &&
    currentHeadSha != null &&
    currentHeadSha !== note.capturedHeadSha;
  const changedRefs: string[] = [];
  const deletedRefs: string[] = [];
  for (const ref of note.capturedRefs) {
    const current = resolveBlobSha(ref.path);
    if (current === null) {
      deletedRefs.push(ref.path);
    } else if (ref.blobSha && ref.blobSha !== current) {
      changedRefs.push(ref.path);
    }
  }
  const status: FreshnessStatus = deletedRefs.length > 0
    ? "very-stale"
    : headAdvanced || changedRefs.length > 0
      ? "stale"
      : "fresh";
  return {
    status,
    headAdvanced,
    changedRefs,
    deletedRefs,
    totalRefs: note.capturedRefs.length,
  };
}
