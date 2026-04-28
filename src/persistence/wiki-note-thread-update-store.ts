import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";

/**
 * Records which thread last touched each wiki note, in service of the
 * per-thread "Finished" list on the rail. Notes are global (one body per
 * slug); attribution is tracked here so the rail can surface only the
 * notes the *current* thread has authored or revised.
 *
 * Mirrors how `work_item_effort` attributes task closures to a thread —
 * the rail merges both feeds in `Runtime.listRecentlyFinished`.
 */
export interface WikiNoteThreadUpdateRow {
  slug: string;
  thread_id: string;
  updated_at: string;
}

export class WikiNoteThreadUpdateStore {
  private readonly stateDb;

  constructor(projectDir: string, logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
  }

  /**
   * Upsert "thread X most recently touched note Y at time T". Caller is
   * the runtime — every entry point that mutates a note funnels through
   * here (resync MCP, `writeWikiNoteBody` IPC, the file watcher).
   */
  recordUpdate(slug: string, threadId: string, at: string = new Date().toISOString()): void {
    this.stateDb.run(
      `INSERT INTO wiki_note_thread_update (slug, thread_id, updated_at)
       VALUES (?, ?, ?)
       ON CONFLICT(slug, thread_id) DO UPDATE SET updated_at = excluded.updated_at`,
      slug,
      threadId,
      at,
    );
  }

  /**
   * Most recent note updates attributed to this thread, newest first.
   * Returns the side-table rows only — caller joins with WikiNoteStore
   * for the title.
   */
  listRecentByThread(threadId: string, limit: number): WikiNoteThreadUpdateRow[] {
    return this.stateDb.all<Record<string, unknown>>(
      `SELECT slug, thread_id, updated_at
         FROM wiki_note_thread_update
        WHERE thread_id = ?
        ORDER BY updated_at DESC
        LIMIT ?`,
      threadId,
      limit,
    ).map((row) => ({
      slug: String(row.slug),
      thread_id: String(row.thread_id),
      updated_at: String(row.updated_at),
    }));
  }

  /** Clear every attribution row for a slug (called when the note is deleted). */
  deleteBySlug(slug: string): void {
    this.stateDb.run(`DELETE FROM wiki_note_thread_update WHERE slug = ?`, slug);
  }
}
