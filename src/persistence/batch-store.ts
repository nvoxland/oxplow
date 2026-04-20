import { existsSync, readFileSync } from "node:fs";
import { randomBytes } from "node:crypto";
import { join } from "node:path";
import { createId } from "../core/ids.js";
import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";
import type { Stream } from "./stream-store.js";

export type BatchStatus = "active" | "queued" | "completed";

export interface Batch {
  id: string;
  stream_id: string;
  title: string;
  status: BatchStatus;
  sort_index: number;
  created_at: string;
  updated_at: string;
  pane_target: string;
  resume_session_id: string;
}

export interface BatchState {
  selectedBatchId: string | null;
  activeBatchId: string | null;
  batches: Batch[];
}

export type BatchChangeKind = "created" | "selected" | "reordered" | "promoted" | "completed" | "resume-updated" | "renamed";

export interface BatchChange {
  streamId: string;
  batchId: string;
  kind: BatchChangeKind;
}

interface PersistedBatchState {
  selectedBatchId: string | null;
  batches: Batch[];
}

export class BatchStore {
  private readonly legacyDir: string;
  private readonly stateDb;
  private readonly emitter: StoreEmitter<BatchChange>;

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.legacyDir = join(projectDir, ".newde", "batches");
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.emitter = new StoreEmitter("batch change", logger);
  }

  subscribe(listener: (change: BatchChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  private emitChange(change: BatchChange): void {
    this.emitter.emit(change);
  }

  ensureStream(stream: Stream): BatchState {
    this.ensureStreamRow(stream);
    this.migrateLegacyIfNeeded(stream.id);
    const existing = this.fetchBatches(stream.id);
    if (existing.length > 0) {
      const selected = this.fetchSelectedBatchId(stream.id) ?? existing[0]?.id ?? null;
      if (selected) this.setSelected(stream.id, selected);
      return {
        selectedBatchId: selected,
        activeBatchId: existing.find((batch) => batch.status === "active")?.id ?? null,
        batches: existing,
      };
    }

    const now = new Date().toISOString();
    const batch: Batch = {
      id: createBatchId(),
      stream_id: stream.id,
      title: "Default",
      status: "active",
      sort_index: 0,
      created_at: now,
      updated_at: now,
      pane_target: stream.panes.working,
      resume_session_id: stream.resume.working_session_id,
    };
    this.insertBatch(batch);
    this.setSelected(stream.id, batch.id);
    return this.list(stream.id);
  }

  list(streamId: string): BatchState {
    const batches = this.fetchBatches(streamId);
    return {
      selectedBatchId: this.fetchSelectedBatchId(streamId) ?? batches[0]?.id ?? null,
      activeBatchId: batches.find((batch) => batch.status === "active")?.id ?? null,
      batches,
    };
  }

  findByPane(paneTarget: string): Batch | undefined {
    const row = this.stateDb.get<Record<string, unknown>>(
      "SELECT * FROM batches WHERE pane_target = ? LIMIT 1",
      paneTarget,
    );
    return row ? rowToBatch(row) : undefined;
  }

  getBatch(streamId: string, batchId: string): Batch | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      "SELECT * FROM batches WHERE stream_id = ? AND id = ? LIMIT 1",
      streamId,
      batchId,
    );
    return row ? rowToBatch(row) : null;
  }

  findById(batchId: string): Batch | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      "SELECT * FROM batches WHERE id = ? LIMIT 1",
      batchId,
    );
    return row ? rowToBatch(row) : null;
  }

  create(stream: Stream, input: { title: string }): BatchState {
    this.ensureStreamRow(stream);
    const title = input.title.trim();
    if (!title) throw new Error("batch title is required");
    const now = new Date().toISOString();
    const existing = this.fetchBatches(stream.id);
    const batch: Batch = {
      id: createBatchId(),
      stream_id: stream.id,
      title,
      status: "queued",
      sort_index: existing.length,
      created_at: now,
      updated_at: now,
      pane_target: `${paneSessionName(stream)}:batch-${createWindowName()}`,
      resume_session_id: "",
    };
    this.insertBatch(batch);
    this.setSelected(stream.id, batch.id);
    this.emitChange({ streamId: stream.id, batchId: batch.id, kind: "created" });
    return this.list(stream.id);
  }

  select(streamId: string, batchId: string): BatchState {
    this.ensureBatchExists(streamId, batchId);
    this.setSelected(streamId, batchId);
    return this.list(streamId);
  }

  reorder(streamId: string, batchId: string, targetIndex: number): BatchState {
    const batches = this.fetchBatches(streamId);
    const currentIndex = batches.findIndex((batch) => batch.id === batchId);
    if (currentIndex < 0) throw new Error(`unknown batch: ${batchId}`);
    const clampedIndex = Math.max(0, Math.min(targetIndex, batches.length - 1));
    if (clampedIndex === currentIndex) return this.list(streamId);
    const reordered = batches.slice();
    const [moved] = reordered.splice(currentIndex, 1);
    reordered.splice(clampedIndex, 0, moved);
    this.stateDb.transaction(() => {
      for (const [index, batch] of reordered.entries()) {
        this.stateDb.run(
          "UPDATE batches SET sort_index = ?, updated_at = ? WHERE id = ?",
          index,
          new Date().toISOString(),
          batch.id,
        );
      }
    });
    return this.list(streamId);
  }

  promote(streamId: string, batchId: string): BatchState {
    const batches = this.fetchBatches(streamId);
    const target = batches.find((batch) => batch.id === batchId);
    if (!target) throw new Error(`unknown batch: ${batchId}`);
    if (target.status === "completed") throw new Error("cannot activate a completed batch");
    const now = new Date().toISOString();
    this.stateDb.transaction(() => {
      for (const batch of batches) {
        const status = batch.id === batchId ? "active" : batch.status === "active" ? "queued" : batch.status;
        this.stateDb.run("UPDATE batches SET status = ?, updated_at = ? WHERE id = ?", status, now, batch.id);
      }
      this.setSelected(streamId, batchId);
    });
    return this.reorder(streamId, batchId, 0);
  }

  complete(streamId: string, batchId: string): BatchState {
    const batches = this.fetchBatches(streamId);
    const target = batches.find((batch) => batch.id === batchId);
    if (!target) throw new Error(`unknown batch: ${batchId}`);
    if (target.status !== "active") throw new Error("only the active batch can be completed");
    const nextQueued = batches.find((batch) => batch.status === "queued" && batch.id !== batchId);
    if (!nextQueued) throw new Error("create another queued batch before completing the active batch");
    const now = new Date().toISOString();
    this.stateDb.transaction(() => {
      this.stateDb.run("UPDATE batches SET status = 'completed', updated_at = ? WHERE id = ?", now, batchId);
      this.stateDb.run("UPDATE batches SET status = 'active', updated_at = ? WHERE id = ?", now, nextQueued.id);
      this.setSelected(streamId, nextQueued.id);
    });
    return this.reorder(streamId, nextQueued.id, 0);
  }

  rename(streamId: string, batchId: string, title: string): Batch {
    this.ensureBatchExists(streamId, batchId);
    const trimmed = title.trim();
    if (!trimmed) throw new Error("batch title is required");
    const now = new Date().toISOString();
    this.stateDb.run(
      "UPDATE batches SET title = ?, updated_at = ? WHERE stream_id = ? AND id = ?",
      trimmed,
      now,
      streamId,
      batchId,
    );
    const updated = this.getBatch(streamId, batchId);
    if (!updated) throw new Error(`unknown batch after rename: ${batchId}`);
    this.emitChange({ streamId, batchId, kind: "renamed" });
    return updated;
  }

  updateResume(streamId: string, batchId: string, sessionId: string): void {
    this.ensureBatchExists(streamId, batchId);
    this.stateDb.run(
      "UPDATE batches SET resume_session_id = ?, updated_at = ? WHERE stream_id = ? AND id = ?",
      sessionId,
      new Date().toISOString(),
      streamId,
      batchId,
    );
    this.emitChange({ streamId, batchId, kind: "resume-updated" });
  }

  private fetchBatches(streamId: string): Batch[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        "SELECT * FROM batches WHERE stream_id = ? ORDER BY sort_index, created_at, id",
        streamId,
      )
      .map(rowToBatch);
  }

  private ensureStreamRow(stream: Stream): void {
    this.stateDb.run(
      `INSERT INTO streams (
        id, title, summary, branch, branch_ref, branch_source, worktree_path,
        working_pane, talking_pane, working_session_id, talking_session_id,
        created_at, updated_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(id) DO NOTHING`,
      stream.id,
      stream.title,
      stream.summary,
      stream.branch,
      stream.branch_ref,
      stream.branch_source,
      stream.worktree_path,
      stream.panes.working,
      stream.panes.talking,
      stream.resume.working_session_id,
      stream.resume.talking_session_id,
      stream.created_at,
      stream.updated_at,
    );
  }

  private fetchSelectedBatchId(streamId: string): string | null {
    const row = this.stateDb.get<{ selected_batch_id: string | null }>(
      "SELECT selected_batch_id FROM batch_selection WHERE stream_id = ? LIMIT 1",
      streamId,
    );
    return row?.selected_batch_id ?? null;
  }

  private insertBatch(batch: Batch): void {
    this.stateDb.run(
      `INSERT INTO batches (
        id, stream_id, title, status, sort_index, pane_target, resume_session_id,
        created_at, updated_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      batch.id,
      batch.stream_id,
      batch.title,
      batch.status,
      batch.sort_index,
      batch.pane_target,
      batch.resume_session_id,
      batch.created_at,
      batch.updated_at,
    );
  }

  private setSelected(streamId: string, batchId: string): void {
    this.stateDb.run(
      `INSERT INTO batch_selection (stream_id, selected_batch_id)
       VALUES (?, ?)
       ON CONFLICT(stream_id) DO UPDATE SET selected_batch_id = excluded.selected_batch_id`,
      streamId,
      batchId,
    );
  }

  private ensureBatchExists(streamId: string, batchId: string): void {
    const row = this.stateDb.get<{ id: string }>(
      "SELECT id FROM batches WHERE stream_id = ? AND id = ? LIMIT 1",
      streamId,
      batchId,
    );
    if (!row) throw new Error(`unknown batch: ${batchId}`);
  }

  private migrateLegacyIfNeeded(streamId: string): void {
    const existing = this.stateDb.get<{ c: number }>("SELECT COUNT(*) AS c FROM batches WHERE stream_id = ?", streamId);
    if ((existing?.c ?? 0) > 0) return;
    const path = join(this.legacyDir, `${streamId}.json`);
    if (!existsSync(path)) return;
    try {
      const legacy = normalizeState(JSON.parse(readFileSync(path, "utf8")) as PersistedBatchState, streamId);
      this.stateDb.transaction(() => {
        for (const batch of legacy.batches) this.insertBatch(batch);
        if (legacy.selectedBatchId) this.setSelected(streamId, legacy.selectedBatchId);
      });
      this.logger?.info("imported legacy batch state into sqlite", {
        streamId,
        count: legacy.batches.length,
      });
    } catch (error) {
      this.logger?.warn("failed to import legacy batch state", {
        streamId,
        error: errorMessage(error),
      });
    }
  }
}

function rowToBatch(row: Record<string, unknown>): Batch {
  return {
    id: String(row.id ?? ""),
    stream_id: String(row.stream_id ?? ""),
    title: String(row.title ?? ""),
    status: String(row.status ?? "queued") as BatchStatus,
    sort_index: Number(row.sort_index ?? 0),
    created_at: String(row.created_at ?? new Date(0).toISOString()),
    updated_at: String(row.updated_at ?? row.created_at ?? new Date(0).toISOString()),
    pane_target: String(row.pane_target ?? ""),
    resume_session_id: String(row.resume_session_id ?? ""),
  };
}

function normalizeState(state: PersistedBatchState, streamId: string): PersistedBatchState {
  const batches = (Array.isArray(state.batches) ? state.batches : [])
    .map((batch, index) => ({
      ...batch,
      stream_id: batch.stream_id || streamId,
      sort_index: Number.isFinite(batch.sort_index) ? batch.sort_index : index,
      resume_session_id: batch.resume_session_id ?? "",
    }))
    .sort((a, b) => a.sort_index - b.sort_index || a.created_at.localeCompare(b.created_at))
    .map((batch, index) => ({ ...batch, sort_index: index }));
  return {
    selectedBatchId: state.selectedBatchId && batches.some((batch) => batch.id === state.selectedBatchId)
      ? state.selectedBatchId
      : batches[0]?.id ?? null,
    batches,
  };
}

function paneSessionName(stream: Stream) {
  return stream.panes.working.split(":")[0];
}

function createBatchId() {
  return createId("b");
}

function createWindowName() {
  return randomBytes(3).toString("hex");
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
