import type { Logger } from "../core/logger.js";
import { createId } from "../core/ids.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";

export type WaitPointStatus = "pending" | "triggered";

const WAIT_POINT_STATUSES: ReadonlySet<WaitPointStatus> = new Set(["pending", "triggered"]);

const NOTE_MAX_LEN = 2_000;

export interface WaitPoint {
  id: string;
  thread_id: string;
  sort_index: number;
  status: WaitPointStatus;
  note: string | null;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
}

export interface WaitPointChange {
  threadId: string;
  kind: "created" | "updated" | "deleted";
  id: string | null;
}

export class WaitPointStore {
  private readonly stateDb;
  private readonly emitter: StoreEmitter<WaitPointChange>;

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.emitter = new StoreEmitter("wait point", logger);
  }

  subscribe(listener: (change: WaitPointChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  private emit(change: WaitPointChange): void {
    this.emitter.emit(change);
  }

  listForThread(threadId: string): WaitPoint[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM wait_point WHERE thread_id = ? ORDER BY sort_index, created_at, id`,
        threadId,
      )
      .map(toWaitPoint);
  }

  get(id: string): WaitPoint | null {
    const row = this.stateDb.get<Record<string, unknown>>(`SELECT * FROM wait_point WHERE id = ?`, id);
    return row ? toWaitPoint(row) : null;
  }

  create(input: { threadId: string; sortIndex: number; note?: string | null }): WaitPoint {
    const id = createId("wp");
    const now = new Date().toISOString();
    const note = input.note?.slice(0, NOTE_MAX_LEN) ?? null;
    this.stateDb.run(
      `INSERT INTO wait_point (id, thread_id, sort_index, status, note, created_at, updated_at)
       VALUES (?, ?, ?, 'pending', ?, ?, ?)`,
      id, input.threadId, input.sortIndex, note, now, now,
    );
    const stored = this.get(id);
    if (!stored) throw new Error("wait point not persisted");
    this.emit({ threadId: input.threadId, kind: "created", id });
    return stored;
  }

  /** Flip `pending` → `triggered` when the agent stops at this point. */
  trigger(id: string): WaitPoint {
    const wp = this.require(id);
    if (wp.status !== "pending") return wp;
    const now = new Date().toISOString();
    this.stateDb.run(`UPDATE wait_point SET status = 'triggered', updated_at = ? WHERE id = ?`, now, id);
    const updated = this.require(id);
    this.emit({ threadId: updated.thread_id, kind: "updated", id });
    return updated;
  }

  /** Bulk assign sort_index values — paired with commit-point + work-item
   *  bulk setters so the thread-queue reorder can keep all three tables in a
   *  single index space. */
  setSortIndexes(entries: Array<{ id: string; sortIndex: number }>): void {
    if (entries.length === 0) return;
    const now = new Date().toISOString();
    this.stateDb.transaction(() => {
      for (const entry of entries) {
        this.stateDb.run(
          `UPDATE wait_point SET sort_index = ?, updated_at = ? WHERE id = ?`,
          entry.sortIndex, now, entry.id,
        );
      }
    });
    const threads = new Set<string>();
    for (const entry of entries) {
      const wp = this.get(entry.id);
      if (wp) threads.add(wp.thread_id);
    }
    for (const threadId of threads) {
      this.emit({ threadId, kind: "updated", id: null });
    }
  }

  setNote(id: string, note: string | null): WaitPoint {
    this.require(id);
    const now = new Date().toISOString();
    const trimmed = note == null ? null : note.slice(0, NOTE_MAX_LEN);
    this.stateDb.run(`UPDATE wait_point SET note = ?, updated_at = ? WHERE id = ?`, trimmed, now, id);
    const updated = this.require(id);
    this.emit({ threadId: updated.thread_id, kind: "updated", id });
    return updated;
  }

  delete(id: string): void {
    const wp = this.get(id);
    if (!wp) return;
    this.stateDb.run(`DELETE FROM wait_point WHERE id = ?`, id);
    this.emit({ threadId: wp.thread_id, kind: "deleted", id });
  }

  private require(id: string): WaitPoint {
    const wp = this.get(id);
    if (!wp) throw new Error(`wait point ${id} not found`);
    return wp;
  }
}

function toWaitPoint(row: Record<string, unknown>): WaitPoint {
  const status = String(row.status);
  if (!WAIT_POINT_STATUSES.has(status as WaitPointStatus)) throw new Error(`invalid wait_point.status: ${status}`);
  return {
    id: String(row.id),
    thread_id: String(row.thread_id),
    sort_index: Number(row.sort_index),
    status: status as WaitPointStatus,
    note: row.note == null ? null : String(row.note),
    created_at: String(row.created_at),
    updated_at: String(row.updated_at),
    completed_at: row.completed_at == null ? null : String(row.completed_at),
  };
}
