import { createId } from "../core/ids.js";
import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";

export type FileChangeKind = "created" | "updated" | "deleted";
export type FileChangeSource = "hook" | "fs-watch";

export interface BatchFileChange {
  id: string;
  batch_id: string;
  turn_id: string | null;
  work_item_id: string | null;
  path: string;
  change_kind: FileChangeKind;
  source: FileChangeSource;
  tool_name: string | null;
  created_at: string;
}

export interface FileChangeInput {
  batchId: string;
  turnId: string | null;
  workItemId: string | null;
  path: string;
  changeKind: FileChangeKind;
  source: FileChangeSource;
  toolName?: string | null;
}

export class FileChangeStore {
  private readonly stateDb;
  private readonly listeners = new Set<(change: BatchFileChange) => void>();

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
  }

  subscribe(listener: (change: BatchFileChange) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private emit(change: BatchFileChange): void {
    for (const listener of this.listeners) {
      try {
        listener(change);
      } catch (error) {
        this.logger?.warn("file change listener threw", {
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }
  }

  record(input: FileChangeInput): BatchFileChange {
    const change: BatchFileChange = {
      id: createId("fc"),
      batch_id: input.batchId,
      turn_id: input.turnId,
      work_item_id: input.workItemId,
      path: input.path,
      change_kind: input.changeKind,
      source: input.source,
      tool_name: input.toolName ?? null,
      created_at: new Date().toISOString(),
    };
    this.stateDb.run(
      `INSERT INTO batch_file_change (
        id, batch_id, turn_id, work_item_id, path, change_kind, source, tool_name, created_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      change.id,
      change.batch_id,
      change.turn_id,
      change.work_item_id,
      change.path,
      change.change_kind,
      change.source,
      change.tool_name,
      change.created_at,
    );
    this.emit(change);
    return change;
  }

  listForBatch(batchId: string, limit = 200): BatchFileChange[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM batch_file_change WHERE batch_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?`,
        batchId,
        limit,
      )
      .map(rowToChange);
  }

  hasChangeForPath(batchId: string, path: string): boolean {
    const row = this.stateDb.get<{ c: number }>(
      `SELECT COUNT(*) AS c FROM batch_file_change WHERE batch_id = ? AND path = ?`,
      batchId,
      path,
    );
    return (row?.c ?? 0) > 0;
  }

  listForTurn(turnId: string): BatchFileChange[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM batch_file_change WHERE turn_id = ? ORDER BY created_at DESC, rowid DESC`,
        turnId,
      )
      .map(rowToChange);
  }
}

function rowToChange(row: Record<string, unknown>): BatchFileChange {
  return {
    id: String(row.id ?? ""),
    batch_id: String(row.batch_id ?? ""),
    turn_id: row.turn_id == null ? null : String(row.turn_id),
    work_item_id: row.work_item_id == null ? null : String(row.work_item_id),
    path: String(row.path ?? ""),
    change_kind: String(row.change_kind ?? "updated") as FileChangeKind,
    source: String(row.source ?? "fs-watch") as FileChangeSource,
    tool_name: row.tool_name == null ? null : String(row.tool_name),
    created_at: String(row.created_at ?? ""),
  };
}
