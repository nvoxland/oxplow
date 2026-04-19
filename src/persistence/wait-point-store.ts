import type { Logger } from "../core/logger.js";
import { createId } from "../core/ids.js";
import { getStateDatabase } from "./state-db.js";

export type WaitPointStatus = "pending" | "triggered";

const WAIT_POINT_STATUSES: ReadonlySet<WaitPointStatus> = new Set(["pending", "triggered"]);

const NOTE_MAX_LEN = 2_000;

export interface WaitPoint {
  id: string;
  batch_id: string;
  sort_index: number;
  status: WaitPointStatus;
  note: string | null;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
}

export interface WaitPointChange {
  batchId: string;
  kind: "created" | "updated" | "deleted";
  id: string | null;
}

export class WaitPointStore {
  private readonly stateDb;
  private readonly listeners = new Set<(change: WaitPointChange) => void>();

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
  }

  subscribe(listener: (change: WaitPointChange) => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  private emit(change: WaitPointChange): void {
    for (const l of this.listeners) {
      try { l(change); } catch (e) {
        this.logger?.warn("wait point listener threw", { error: e instanceof Error ? e.message : String(e) });
      }
    }
  }

  listForBatch(batchId: string): WaitPoint[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM wait_point WHERE batch_id = ? ORDER BY sort_index, created_at, id`,
        batchId,
      )
      .map(toWaitPoint);
  }

  get(id: string): WaitPoint | null {
    const row = this.stateDb.get<Record<string, unknown>>(`SELECT * FROM wait_point WHERE id = ?`, id);
    return row ? toWaitPoint(row) : null;
  }

  create(input: { batchId: string; sortIndex: number; note?: string | null }): WaitPoint {
    const id = createId("wp");
    const now = new Date().toISOString();
    const note = input.note?.slice(0, NOTE_MAX_LEN) ?? null;
    this.stateDb.run(
      `INSERT INTO wait_point (id, batch_id, sort_index, status, note, created_at, updated_at)
       VALUES (?, ?, ?, 'pending', ?, ?, ?)`,
      id, input.batchId, input.sortIndex, note, now, now,
    );
    const stored = this.get(id);
    if (!stored) throw new Error("wait point not persisted");
    this.emit({ batchId: input.batchId, kind: "created", id });
    return stored;
  }

  /** Flip `pending` → `triggered` when the agent stops at this point. */
  trigger(id: string): WaitPoint {
    const wp = this.require(id);
    if (wp.status !== "pending") return wp;
    const now = new Date().toISOString();
    this.stateDb.run(`UPDATE wait_point SET status = 'triggered', updated_at = ? WHERE id = ?`, now, id);
    const updated = this.require(id);
    this.emit({ batchId: updated.batch_id, kind: "updated", id });
    return updated;
  }

  /** Bulk assign sort_index values — paired with commit-point + work-item
   *  bulk setters so the batch-queue reorder can keep all three tables in a
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
    const batches = new Set<string>();
    for (const entry of entries) {
      const wp = this.get(entry.id);
      if (wp) batches.add(wp.batch_id);
    }
    for (const batchId of batches) {
      this.emit({ batchId, kind: "updated", id: null });
    }
  }

  setNote(id: string, note: string | null): WaitPoint {
    this.require(id);
    const now = new Date().toISOString();
    const trimmed = note == null ? null : note.slice(0, NOTE_MAX_LEN);
    this.stateDb.run(`UPDATE wait_point SET note = ?, updated_at = ? WHERE id = ?`, trimmed, now, id);
    const updated = this.require(id);
    this.emit({ batchId: updated.batch_id, kind: "updated", id });
    return updated;
  }

  delete(id: string): void {
    const wp = this.get(id);
    if (!wp) return;
    this.stateDb.run(`DELETE FROM wait_point WHERE id = ?`, id);
    this.emit({ batchId: wp.batch_id, kind: "deleted", id });
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
    batch_id: String(row.batch_id),
    sort_index: Number(row.sort_index),
    status: status as WaitPointStatus,
    note: row.note == null ? null : String(row.note),
    created_at: String(row.created_at),
    updated_at: String(row.updated_at),
    completed_at: row.completed_at == null ? null : String(row.completed_at),
  };
}
