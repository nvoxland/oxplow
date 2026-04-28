/**
 * In-memory store for "what's running right now" rows surfaced in the
 * status bar. Modeled on FollowupStore — no SQLite, lost on restart.
 *
 * Producers (git ops, code-quality scans, LSP startup, notes resync)
 * call `start()` to register a row, optionally `update()` for progress
 * ticks, and `complete()` / `fail()` when finished. Done/failed rows
 * stick around for a brief grace window so the UI can flash a checkmark
 * before the row disappears.
 */
import { StoreEmitter } from "../persistence/store-emitter.js";
import type { Logger } from "../core/logger.js";

export type BackgroundTaskKind = "git" | "code-quality" | "lsp" | "notes-resync";
export type BackgroundTaskStatus = "running" | "done" | "failed";

export interface BackgroundTask {
  id: string;
  kind: BackgroundTaskKind;
  label: string;
  detail?: string;
  /** 0..1 for determinate, null for indeterminate. */
  progress: number | null;
  status: BackgroundTaskStatus;
  startedAt: number;
  endedAt?: number;
  error?: string;
  /** Producer-supplied opaque payload attached at complete/fail time so
   *  consumers awaiting the task by id can read the final result (e.g.
   *  the GitOpResult) without a separate channel. */
  result?: unknown;
}

export interface BackgroundTaskChange {
  kind: "started" | "updated" | "ended";
  id: string;
}

export interface BackgroundTaskStartInput {
  kind: BackgroundTaskKind;
  label: string;
  detail?: string;
  progress?: number | null;
}

export interface BackgroundTaskUpdateInput {
  label?: string;
  detail?: string;
  progress?: number | null;
}

const DEFAULT_GRACE_MS = 4000;

export class BackgroundTaskStore {
  private readonly tasks = new Map<string, BackgroundTask>();
  private readonly evictTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly emitter: StoreEmitter<BackgroundTaskChange>;
  private nextSeq = 1;

  constructor(
    logger?: Logger,
    private readonly graceMs = DEFAULT_GRACE_MS,
  ) {
    this.emitter = new StoreEmitter<BackgroundTaskChange>("background-task-store", logger);
  }

  start(input: BackgroundTaskStartInput): string {
    const id = `bg-${this.nextSeq++}-${Math.random().toString(36).slice(2, 8)}`;
    const task: BackgroundTask = {
      id,
      kind: input.kind,
      label: input.label,
      detail: input.detail,
      progress: clampProgress(input.progress ?? null),
      status: "running",
      startedAt: Date.now(),
    };
    this.tasks.set(id, task);
    this.emitter.emit({ kind: "started", id });
    return id;
  }

  update(id: string, patch: BackgroundTaskUpdateInput): void {
    const task = this.tasks.get(id);
    if (!task || task.status !== "running") return;
    if (patch.label !== undefined) task.label = patch.label;
    if (patch.detail !== undefined) task.detail = patch.detail;
    if (patch.progress !== undefined) task.progress = clampProgress(patch.progress);
    this.emitter.emit({ kind: "updated", id });
  }

  complete(id: string, result?: unknown): void {
    const task = this.tasks.get(id);
    if (!task || task.status !== "running") return;
    task.status = "done";
    task.endedAt = Date.now();
    if (task.progress !== null) task.progress = 1;
    if (result !== undefined) task.result = result;
    this.scheduleEviction(id);
    this.emitter.emit({ kind: "ended", id });
  }

  fail(id: string, errorMessage: string, result?: unknown): void {
    const task = this.tasks.get(id);
    if (!task || task.status !== "running") return;
    task.status = "failed";
    task.error = errorMessage;
    task.endedAt = Date.now();
    if (result !== undefined) task.result = result;
    this.scheduleEviction(id);
    this.emitter.emit({ kind: "ended", id });
  }

  get(id: string): BackgroundTask | null {
    const task = this.tasks.get(id);
    return task ? { ...task } : null;
  }

  list(): BackgroundTask[] {
    const arr = [...this.tasks.values()];
    arr.sort((a, b) => {
      if (a.status === "running" && b.status !== "running") return -1;
      if (a.status !== "running" && b.status === "running") return 1;
      return a.startedAt - b.startedAt;
    });
    return arr.map((t) => ({ ...t }));
  }

  subscribe(listener: (change: BackgroundTaskChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  dispose(): void {
    for (const timer of this.evictTimers.values()) clearTimeout(timer);
    this.evictTimers.clear();
    this.tasks.clear();
  }

  private scheduleEviction(id: string): void {
    const existing = this.evictTimers.get(id);
    if (existing) clearTimeout(existing);
    const timer = setTimeout(() => {
      this.evictTimers.delete(id);
      if (this.tasks.delete(id)) this.emitter.emit({ kind: "ended", id });
    }, this.graceMs);
    if (typeof timer === "object" && timer && "unref" in timer && typeof timer.unref === "function") {
      timer.unref();
    }
    this.evictTimers.set(id, timer);
  }
}

function clampProgress(p: number | null): number | null {
  if (p === null) return null;
  if (!Number.isFinite(p)) return null;
  if (p < 0) return 0;
  if (p > 1) return 1;
  return p;
}
