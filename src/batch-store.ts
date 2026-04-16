import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { randomBytes } from "node:crypto";
import { join } from "node:path";
import type { Logger } from "./logger.js";
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

interface PersistedBatchState {
  selectedBatchId: string | null;
  batches: Batch[];
}

export class BatchStore {
  private readonly rootDir: string;
  private readonly dir: string;

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.rootDir = join(projectDir, ".newde");
    this.dir = join(this.rootDir, "batches");
    mkdirSync(this.rootDir, { recursive: true });
    mkdirSync(this.dir, { recursive: true });
  }

  ensureStream(stream: Stream): BatchState {
    const existing = this.readState(stream.id);
    if (existing.batches.length > 0) {
      const selected = existing.selectedBatchId ?? existing.batches[0]?.id ?? null;
      if (selected !== existing.selectedBatchId) {
        this.writeState(stream.id, { ...existing, selectedBatchId: selected });
      }
      return toBatchState({ ...existing, selectedBatchId: selected });
    }

    const now = new Date().toISOString();
    const initial: Batch = {
      id: createBatchId(),
      stream_id: stream.id,
      title: "Current Batch",
      status: "active",
      sort_index: 0,
      created_at: now,
      updated_at: now,
      pane_target: stream.panes.working,
      resume_session_id: stream.resume.working_session_id,
    };
    const state = { selectedBatchId: initial.id, batches: [initial] };
    this.writeState(stream.id, state);
    return toBatchState(state);
  }

  list(streamId: string): BatchState {
    return toBatchState(this.readState(streamId));
  }

  findByPane(paneTarget: string): Batch | undefined {
    for (const fileName of readdirSync(this.dir)) {
      if (!fileName.endsWith(".json")) continue;
      const state = readPersistedState(join(this.dir, fileName));
      const batch = state.batches.find((candidate) => candidate.pane_target === paneTarget);
      if (batch) return batch;
    }
    return undefined;
  }

  create(stream: Stream, input: { title: string }): BatchState {
    const title = input.title.trim();
    if (!title) throw new Error("batch title is required");
    const state = this.readState(stream.id);
    const now = new Date().toISOString();
    const batch: Batch = {
      id: createBatchId(),
      stream_id: stream.id,
      title,
      status: "queued",
      sort_index: state.batches.length,
      created_at: now,
      updated_at: now,
      pane_target: `${paneSessionName(stream)}:batch-${createWindowName()}`,
      resume_session_id: "",
    };
    const next = {
      selectedBatchId: batch.id,
      batches: [...state.batches, batch],
    };
    this.writeState(stream.id, next);
    return toBatchState(next);
  }

  select(streamId: string, batchId: string): BatchState {
    const state = this.readState(streamId);
    this.ensureBatchExists(state, batchId);
    const next = { ...state, selectedBatchId: batchId };
    this.writeState(streamId, next);
    return toBatchState(next);
  }

  reorder(streamId: string, batchId: string, targetIndex: number): BatchState {
    const state = this.readState(streamId);
    const currentIndex = state.batches.findIndex((batch) => batch.id === batchId);
    if (currentIndex < 0) throw new Error(`unknown batch: ${batchId}`);
    const clampedIndex = Math.max(0, Math.min(targetIndex, state.batches.length - 1));
    if (clampedIndex === currentIndex) return toBatchState(state);
    const reordered = state.batches.slice();
    const [moved] = reordered.splice(currentIndex, 1);
    reordered.splice(clampedIndex, 0, moved);
    const now = new Date().toISOString();
    const next = {
      selectedBatchId: state.selectedBatchId,
      batches: reordered.map((batch, index) => ({
        ...batch,
        sort_index: index,
        updated_at: now,
      })),
    };
    this.writeState(streamId, next);
    return toBatchState(next);
  }

  promote(streamId: string, batchId: string): BatchState {
    const state = this.readState(streamId);
    const target = state.batches.find((batch) => batch.id === batchId);
    if (!target) throw new Error(`unknown batch: ${batchId}`);
    if (target.status === "completed") throw new Error("cannot activate a completed batch");
    const now = new Date().toISOString();
    const next = {
      selectedBatchId: batchId,
      batches: state.batches.map((batch) => ({
        ...batch,
        status: batch.id === batchId ? "active" : batch.status === "active" ? "queued" : batch.status,
        updated_at: now,
      })),
    };
    this.writeState(streamId, next);
    return this.reorder(streamId, batchId, 0);
  }

  complete(streamId: string, batchId: string): BatchState {
    const state = this.readState(streamId);
    const target = state.batches.find((batch) => batch.id === batchId);
    if (!target) throw new Error(`unknown batch: ${batchId}`);
    if (target.status !== "active") throw new Error("only the active batch can be completed");
    const nextQueued = state.batches.find((batch) => batch.status === "queued" && batch.id !== batchId);
    if (!nextQueued) {
      throw new Error("create another queued batch before completing the active batch");
    }
    const now = new Date().toISOString();
    const next = {
      selectedBatchId: nextQueued.id,
      batches: state.batches.map((batch) => ({
        ...batch,
        status: batch.id === batchId ? "completed" : batch.id === nextQueued.id ? "active" : batch.status,
        updated_at: now,
      })),
    };
    this.writeState(streamId, next);
    return this.reorder(streamId, nextQueued.id, 0);
  }

  updateResume(streamId: string, batchId: string, sessionId: string): void {
    const state = this.readState(streamId);
    this.ensureBatchExists(state, batchId);
    const now = new Date().toISOString();
    const next = {
      ...state,
      batches: state.batches.map((batch) =>
        batch.id === batchId
          ? { ...batch, resume_session_id: sessionId, updated_at: now }
          : batch,
      ),
    };
    this.writeState(streamId, next);
  }

  private ensureBatchExists(state: PersistedBatchState, batchId: string): void {
    if (!state.batches.some((batch) => batch.id === batchId)) {
      throw new Error(`unknown batch: ${batchId}`);
    }
  }

  private readState(streamId: string): PersistedBatchState {
    const path = this.pathFor(streamId);
    if (!existsSync(path)) return { selectedBatchId: null, batches: [] };
    return normalizeState(readPersistedState(path), streamId);
  }

  private writeState(streamId: string, state: PersistedBatchState): void {
    const normalized = normalizeState(state, streamId);
    writeFileSync(this.pathFor(streamId), JSON.stringify(normalized, null, 2) + "\n", "utf8");
    this.logger?.info("saved batch state", {
      streamId,
      count: normalized.batches.length,
      selectedBatchId: normalized.selectedBatchId,
    });
  }

  private pathFor(streamId: string): string {
    return join(this.dir, `${streamId}.json`);
  }
}

function readPersistedState(path: string): PersistedBatchState {
  const parsed = JSON.parse(readFileSync(path, "utf8")) as PersistedBatchState;
  return {
    selectedBatchId: parsed.selectedBatchId ?? null,
    batches: Array.isArray(parsed.batches) ? parsed.batches : [],
  };
}

function normalizeState(state: PersistedBatchState, streamId: string): PersistedBatchState {
  const batches = state.batches
    .map((batch, index) => ({
      ...batch,
      stream_id: batch.stream_id || streamId,
      sort_index: Number.isFinite(batch.sort_index) ? batch.sort_index : index,
      resume_session_id: batch.resume_session_id ?? "",
    }))
    .sort((a, b) => a.sort_index - b.sort_index || a.created_at.localeCompare(b.created_at));

  const selectedBatchId = state.selectedBatchId && batches.some((batch) => batch.id === state.selectedBatchId)
    ? state.selectedBatchId
    : batches[0]?.id ?? null;

  return {
    selectedBatchId,
    batches: batches.map((batch, index) => ({
      ...batch,
      sort_index: index,
    })),
  };
}

function toBatchState(state: PersistedBatchState): BatchState {
  return {
    selectedBatchId: state.selectedBatchId,
    activeBatchId: state.batches.find((batch) => batch.status === "active")?.id ?? null,
    batches: state.batches,
  };
}

function paneSessionName(stream: Stream) {
  return stream.panes.working.split(":")[0];
}

function createBatchId() {
  return "b-" + randomBytes(4).toString("hex");
}

function createWindowName() {
  return randomBytes(3).toString("hex");
}
