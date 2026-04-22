import { createId } from "../core/ids.js";
import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";

export interface WorkItemEffort {
  id: string;
  work_item_id: string;
  started_at: string;
  ended_at: string | null;
  start_snapshot_id: string | null;
  end_snapshot_id: string | null;
}

export type EffortChangeKind = "opened" | "closed";

export interface EffortChange {
  effortId: string;
  workItemId: string;
  kind: EffortChangeKind;
}

export class WorkItemEffortStore {
  private readonly stateDb;
  private readonly emitter: StoreEmitter<EffortChange>;

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.emitter = new StoreEmitter("effort change", logger);
  }

  subscribe(listener: (change: EffortChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  /**
   * Open a new effort for a work item. Returns the existing open effort if
   * one already exists (idempotent so status-transition hooks can safely
   * retry).
   */
  openEffort(input: { workItemId: string; startSnapshotId: string | null }): WorkItemEffort {
    const existing = this.getOpenEffort(input.workItemId);
    if (existing) {
      if (input.startSnapshotId && !existing.start_snapshot_id) {
        this.stateDb.run(
          `UPDATE work_item_effort SET start_snapshot_id = ? WHERE id = ?`,
          input.startSnapshotId,
          existing.id,
        );
        return { ...existing, start_snapshot_id: input.startSnapshotId };
      }
      return existing;
    }
    const id = createId("eff");
    const now = new Date().toISOString();
    this.stateDb.run(
      `INSERT INTO work_item_effort (id, work_item_id, started_at, start_snapshot_id)
       VALUES (?, ?, ?, ?)`,
      id,
      input.workItemId,
      now,
      input.startSnapshotId,
    );
    const row = this.getById(id);
    if (!row) throw new Error("failed to read back inserted effort");
    this.emitter.emit({ effortId: id, workItemId: input.workItemId, kind: "opened" });
    return row;
  }

  /**
   * Close the currently open effort for a work item. No-op if no effort is
   * open. Returns the closed row (or null).
   */
  closeEffort(input: { workItemId: string; endSnapshotId: string | null }): WorkItemEffort | null {
    const existing = this.getOpenEffort(input.workItemId);
    if (!existing) return null;
    const now = new Date().toISOString();
    this.stateDb.run(
      `UPDATE work_item_effort SET ended_at = ?, end_snapshot_id = ? WHERE id = ?`,
      now,
      input.endSnapshotId,
      existing.id,
    );
    const row = this.getById(existing.id);
    if (row) {
      this.emitter.emit({ effortId: existing.id, workItemId: input.workItemId, kind: "closed" });
    }
    return row;
  }

  getById(id: string): WorkItemEffort | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT * FROM work_item_effort WHERE id = ? LIMIT 1`,
      id,
    );
    return row ? rowToEffort(row) : null;
  }

  getOpenEffort(workItemId: string): WorkItemEffort | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT * FROM work_item_effort WHERE work_item_id = ? AND ended_at IS NULL
       ORDER BY started_at DESC, rowid DESC LIMIT 1`,
      workItemId,
    );
    return row ? rowToEffort(row) : null;
  }

  listOpenEfforts(): WorkItemEffort[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM work_item_effort WHERE ended_at IS NULL ORDER BY started_at ASC`,
      )
      .map(rowToEffort);
  }

  listEffortsForWorkItem(workItemId: string): WorkItemEffort[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM work_item_effort WHERE work_item_id = ? ORDER BY started_at ASC, rowid ASC`,
        workItemId,
      )
      .map(rowToEffort);
  }

  listEffortsForSnapshot(snapshotId: string): WorkItemEffort[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM work_item_effort
         WHERE start_snapshot_id = ? OR end_snapshot_id = ?
         ORDER BY started_at ASC`,
        snapshotId,
        snapshotId,
      )
      .map(rowToEffort);
  }

  linkEffortTurn(effortId: string, turnId: string): void {
    this.stateDb.run(
      `INSERT OR IGNORE INTO work_item_effort_turn (effort_id, turn_id) VALUES (?, ?)`,
      effortId,
      turnId,
    );
  }

  listTurnsForEffort(effortId: string): string[] {
    const rows = this.stateDb.all<{ turn_id: string }>(
      `SELECT turn_id FROM work_item_effort_turn WHERE effort_id = ? ORDER BY rowid ASC`,
      effortId,
    );
    return rows.map((row) => row.turn_id);
  }

  listEffortsForTurn(turnId: string): WorkItemEffort[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT e.* FROM work_item_effort e
         JOIN work_item_effort_turn link ON link.effort_id = e.id
         WHERE link.turn_id = ?
         ORDER BY e.started_at ASC`,
        turnId,
      )
      .map(rowToEffort);
  }
}

function rowToEffort(row: Record<string, unknown>): WorkItemEffort {
  return {
    id: String(row.id ?? ""),
    work_item_id: String(row.work_item_id ?? ""),
    started_at: String(row.started_at ?? ""),
    ended_at: row.ended_at == null ? null : String(row.ended_at),
    start_snapshot_id: row.start_snapshot_id == null ? null : String(row.start_snapshot_id),
    end_snapshot_id: row.end_snapshot_id == null ? null : String(row.end_snapshot_id),
  };
}
