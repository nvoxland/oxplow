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
/** Snapshots of completed tasks survive UI eviction by this much longer so
 *  late `awaitDone` lookups can still retrieve `result`/`error` after the
 *  in-flight row has been pruned from the status bar. Long-tail safety net. */
const SNAPSHOT_RETENTION_MS = 5 * 60_000;
const SNAPSHOT_MAX_ENTRIES = 200;

export class BackgroundTaskStore {
  private readonly tasks = new Map<string, BackgroundTask>();
  private readonly evictTimers = new Map<string, ReturnType<typeof setTimeout>>();
  /** Frozen snapshots of completed/failed tasks, retained well past
   *  `graceMs` so callers awaiting `getBackgroundTask(id)` after the
   *  row has been evicted still get the final `result` / `error`.
   *  This was added after a `git rebase` op that succeeded silently
   *  surfaced an op-error page with no stderr/stdout/exitCode — the
   *  renderer's `awaitBackgroundTask` lookup raced eviction. */
  private readonly snapshots = new Map<string, BackgroundTask>();
  private readonly snapshotTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly emitter: StoreEmitter<BackgroundTaskChange>;
  private nextSeq = 1;

  constructor(
    logger?: Logger,
    private readonly graceMs = DEFAULT_GRACE_MS,
    private readonly snapshotRetentionMs = SNAPSHOT_RETENTION_MS,
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
    this.snapshot(task);
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
    this.snapshot(task);
    this.scheduleEviction(id);
    this.emitter.emit({ kind: "ended", id });
  }

  /** Returns the live row, falling back to the post-eviction snapshot so
   *  late `awaitDone` lookups still see `result` / `error`. */
  get(id: string): BackgroundTask | null {
    const task = this.tasks.get(id) ?? this.snapshots.get(id);
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

  /** Snapshot of currently-running tasks. Used by the quit confirmation
   *  dialog to surface a list of what's still in flight. */
  listRunning(): BackgroundTask[] {
    const out: BackgroundTask[] = [];
    for (const t of this.tasks.values()) {
      if (t.status === "running") out.push({ ...t });
    }
    out.sort((a, b) => a.startedAt - b.startedAt);
    return out;
  }

  subscribe(listener: (change: BackgroundTaskChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  dispose(): void {
    for (const timer of this.evictTimers.values()) clearTimeout(timer);
    this.evictTimers.clear();
    for (const timer of this.snapshotTimers.values()) clearTimeout(timer);
    this.snapshotTimers.clear();
    this.tasks.clear();
    this.snapshots.clear();
  }

  private snapshot(task: BackgroundTask): void {
    this.snapshots.set(task.id, { ...task });
    const existing = this.snapshotTimers.get(task.id);
    if (existing) clearTimeout(existing);
    const timer = setTimeout(() => {
      this.snapshotTimers.delete(task.id);
      this.snapshots.delete(task.id);
    }, this.snapshotRetentionMs);
    if (typeof timer === "object" && timer && "unref" in timer && typeof timer.unref === "function") {
      timer.unref();
    }
    this.snapshotTimers.set(task.id, timer);
    // LRU cap: drop the oldest snapshot if we're over budget.
    if (this.snapshots.size > SNAPSHOT_MAX_ENTRIES) {
      const oldest = this.snapshots.keys().next().value;
      if (oldest && oldest !== task.id) {
        this.snapshots.delete(oldest);
        const t = this.snapshotTimers.get(oldest);
        if (t) clearTimeout(t);
        this.snapshotTimers.delete(oldest);
      }
    }
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
