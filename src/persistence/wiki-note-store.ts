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
  capturedHeadSha: string | null;
  capturedRefs: NoteRefSnapshot[];
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
         SET title = ?, captured_head_sha = ?, captured_refs_json = ?, updated_at = ?
         WHERE id = ?`,
        input.title,
        input.capturedHeadSha,
        refsJson,
        now,
        existing.id,
      );
    } else {
      const id = createId("wn");
      this.stateDb.run(
        `INSERT INTO wiki_note (id, slug, title, captured_head_sha, captured_refs_json, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)`,
        id,
        input.slug,
        input.title,
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
