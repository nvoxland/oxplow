import { existsSync, readFileSync } from "node:fs";
import { randomBytes } from "node:crypto";
import { join } from "node:path";
import { createId } from "../core/ids.js";
import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";
import type { Stream } from "./stream-store.js";

export type ThreadStatus = "active" | "queued";

export interface Thread {
  id: string;
  stream_id: string;
  title: string;
  status: ThreadStatus;
  sort_index: number;
  created_at: string;
  updated_at: string;
  pane_target: string;
  resume_session_id: string;
  custom_prompt: string | null;
  closed_at: string | null;
}

export interface ThreadState {
  selectedThreadId: string | null;
  activeThreadId: string | null;
  threads: Thread[];
}

export type ThreadChangeKind = "created" | "selected" | "reordered" | "promoted" | "closed" | "reopened" | "resume-updated" | "renamed" | "prompt-changed";

export interface ThreadChange {
  streamId: string;
  threadId: string;
  kind: ThreadChangeKind;
}

interface PersistedThreadState {
  selectedThreadId: string | null;
  threads: Thread[];
}

export class ThreadStore {
  private readonly legacyDir: string;
  private readonly stateDb;
  private readonly emitter: StoreEmitter<ThreadChange>;

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.legacyDir = join(projectDir, ".oxplow", "threads");
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.emitter = new StoreEmitter("thread change", logger);
  }

  subscribe(listener: (change: ThreadChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  private emitChange(change: ThreadChange): void {
    this.emitter.emit(change);
  }

  ensureStream(stream: Stream): ThreadState {
    this.ensureStreamRow(stream);
    this.migrateLegacyIfNeeded(stream.id);
    const existing = this.fetchThreads(stream.id);
    if (existing.length > 0) {
      const selected = this.fetchSelectedThreadId(stream.id) ?? existing[0]?.id ?? null;
      if (selected) this.setSelected(stream.id, selected);
      return {
        selectedThreadId: selected,
        activeThreadId: existing.find((thread) => thread.status === "active")?.id ?? null,
        threads: existing,
      };
    }

    const now = new Date().toISOString();
    const thread: Thread = {
      id: createThreadId(),
      stream_id: stream.id,
      title: "Default",
      status: "active",
      sort_index: 0,
      created_at: now,
      updated_at: now,
      pane_target: stream.panes.working,
      resume_session_id: stream.resume.working_session_id,
      custom_prompt: null,
      closed_at: null,
    };
    this.insertThread(thread);
    this.setSelected(stream.id, thread.id);
    return this.list(stream.id);
  }

  list(streamId: string): ThreadState {
    const threads = this.fetchThreads(streamId);
    return {
      selectedThreadId: this.fetchSelectedThreadId(streamId) ?? threads[0]?.id ?? null,
      activeThreadId: threads.find((thread) => thread.status === "active")?.id ?? null,
      threads,
    };
  }

  findByPane(paneTarget: string): Thread | undefined {
    const row = this.stateDb.get<Record<string, unknown>>(
      "SELECT * FROM threads WHERE pane_target = ? LIMIT 1",
      paneTarget,
    );
    return row ? rowToThread(row) : undefined;
  }

  getThread(streamId: string, threadId: string): Thread | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      "SELECT * FROM threads WHERE stream_id = ? AND id = ? LIMIT 1",
      streamId,
      threadId,
    );
    return row ? rowToThread(row) : null;
  }

  findById(threadId: string): Thread | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      "SELECT * FROM threads WHERE id = ? LIMIT 1",
      threadId,
    );
    return row ? rowToThread(row) : null;
  }

  create(stream: Stream, input: { title: string }): ThreadState {
    this.ensureStreamRow(stream);
    const title = input.title.trim();
    if (!title) throw new Error("thread title is required");
    const now = new Date().toISOString();
    const existing = this.fetchThreads(stream.id);
    const thread: Thread = {
      id: createThreadId(),
      stream_id: stream.id,
      title,
      status: "queued",
      sort_index: existing.length,
      created_at: now,
      updated_at: now,
      pane_target: `${paneSessionName(stream)}:thread-${createWindowName()}`,
      resume_session_id: "",
      custom_prompt: null,
      closed_at: null,
    };
    this.insertThread(thread);
    this.setSelected(stream.id, thread.id);
    this.emitChange({ streamId: stream.id, threadId: thread.id, kind: "created" });
    return this.list(stream.id);
  }

  select(streamId: string, threadId: string): ThreadState {
    this.ensureThreadExists(streamId, threadId);
    this.setSelected(streamId, threadId);
    return this.list(streamId);
  }

  reorder(streamId: string, threadId: string, targetIndex: number): ThreadState {
    const threads = this.fetchThreads(streamId);
    const currentIndex = threads.findIndex((thread) => thread.id === threadId);
    if (currentIndex < 0) throw new Error(`unknown thread: ${threadId}`);
    const clampedIndex = Math.max(0, Math.min(targetIndex, threads.length - 1));
    if (clampedIndex === currentIndex) return this.list(streamId);
    const reordered = threads.slice();
    const [moved] = reordered.splice(currentIndex, 1);
    reordered.splice(clampedIndex, 0, moved);
    this.stateDb.transaction(() => {
      for (const [index, thread] of reordered.entries()) {
        this.stateDb.run(
          "UPDATE threads SET sort_index = ?, updated_at = ? WHERE id = ?",
          index,
          new Date().toISOString(),
          thread.id,
        );
      }
    });
    return this.list(streamId);
  }

  promote(streamId: string, threadId: string): ThreadState {
    const threads = this.fetchThreads(streamId);
    const target = threads.find((thread) => thread.id === threadId);
    if (!target) throw new Error(`unknown thread: ${threadId}`);
    if (target.closed_at) throw new Error("cannot activate a closed thread");
    const now = new Date().toISOString();
    this.stateDb.transaction(() => {
      for (const thread of threads) {
        const status = thread.id === threadId ? "active" : thread.status === "active" ? "queued" : thread.status;
        this.stateDb.run("UPDATE threads SET status = ?, updated_at = ? WHERE id = ?", status, now, thread.id);
      }
      this.setSelected(streamId, threadId);
    });
    this.emitChange({ streamId, threadId, kind: "promoted" });
    return this.list(streamId);
  }

  /** Close a thread. Allowed only on non-writer threads with no work
   *  items in `ready` / `blocked` / `in_progress`. The thread keeps its
   *  status (`queued`) but `closed_at` is set, which hides it from the
   *  rail's main list. Reopen via `reopen()`. */
  close(streamId: string, threadId: string): ThreadState {
    const target = this.getThread(streamId, threadId);
    if (!target) throw new Error(`unknown thread: ${threadId}`);
    if (target.closed_at) throw new Error("thread is already closed");
    if (target.status === "active") {
      throw new Error("cannot close the writer thread — promote another thread first");
    }
    const blocking = this.stateDb.get<{ c: number }>(
      `SELECT COUNT(*) AS c FROM work_items
       WHERE thread_id = ?
         AND deleted_at IS NULL
         AND status IN ('ready','blocked','in_progress')`,
      threadId,
    );
    if ((blocking?.c ?? 0) > 0) {
      throw new Error("thread has open work items — finish or move them before closing");
    }
    const now = new Date().toISOString();
    this.stateDb.run("UPDATE threads SET closed_at = ?, updated_at = ? WHERE id = ?", now, now, threadId);
    this.emitChange({ streamId, threadId, kind: "closed" });
    return this.list(streamId);
  }

  /** Reopen a closed thread. Clears `closed_at`; the thread reappears in
   *  the rail as a queued (read-only) thread. */
  reopen(streamId: string, threadId: string): ThreadState {
    const target = this.getThread(streamId, threadId);
    if (!target) throw new Error(`unknown thread: ${threadId}`);
    if (!target.closed_at) throw new Error("thread is not closed");
    const now = new Date().toISOString();
    this.stateDb.run("UPDATE threads SET closed_at = NULL, updated_at = ? WHERE id = ?", now, threadId);
    this.emitChange({ streamId, threadId, kind: "reopened" });
    return this.list(streamId);
  }

  /** List closed threads for the Closed Threads page, newest-closed first. */
  listClosed(streamId: string): Thread[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        "SELECT * FROM threads WHERE stream_id = ? AND closed_at IS NOT NULL ORDER BY closed_at DESC, id",
        streamId,
      )
      .map(rowToThread);
  }

  reorderThreads(streamId: string, orderedThreadIds: string[]): void {
    const now = new Date().toISOString();
    this.stateDb.transaction(() => {
      for (const [index, id] of orderedThreadIds.entries()) {
        this.stateDb.run(
          "UPDATE threads SET sort_index = ?, updated_at = ? WHERE stream_id = ? AND id = ?",
          index,
          now,
          streamId,
          id,
        );
      }
    });
    this.emitChange({ streamId, threadId: orderedThreadIds[0] ?? "", kind: "reordered" });
  }

  rename(streamId: string, threadId: string, title: string): Thread {
    this.ensureThreadExists(streamId, threadId);
    const trimmed = title.trim();
    if (!trimmed) throw new Error("thread title is required");
    const now = new Date().toISOString();
    this.stateDb.run(
      "UPDATE threads SET title = ?, updated_at = ? WHERE stream_id = ? AND id = ?",
      trimmed,
      now,
      streamId,
      threadId,
    );
    const updated = this.getThread(streamId, threadId);
    if (!updated) throw new Error(`unknown thread after rename: ${threadId}`);
    this.emitChange({ streamId, threadId, kind: "renamed" });
    return updated;
  }

  updateResume(streamId: string, threadId: string, sessionId: string): void {
    this.ensureThreadExists(streamId, threadId);
    this.stateDb.run(
      "UPDATE threads SET resume_session_id = ?, updated_at = ? WHERE stream_id = ? AND id = ?",
      sessionId,
      new Date().toISOString(),
      streamId,
      threadId,
    );
    this.emitChange({ streamId, threadId, kind: "resume-updated" });
  }

  setThreadPrompt(threadId: string, prompt: string | null): Thread[] {
    const thread = this.findById(threadId);
    if (!thread) throw new Error(`unknown thread: ${threadId}`);
    this.stateDb.run(
      "UPDATE threads SET custom_prompt = ?, updated_at = ? WHERE id = ?",
      prompt,
      new Date().toISOString(),
      threadId,
    );
    this.emitChange({ streamId: thread.stream_id, threadId, kind: "prompt-changed" });
    return this.fetchThreads(thread.stream_id);
  }

  private fetchThreads(streamId: string): Thread[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        "SELECT * FROM threads WHERE stream_id = ? AND closed_at IS NULL ORDER BY sort_index, created_at, id",
        streamId,
      )
      .map(rowToThread);
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

  private fetchSelectedThreadId(streamId: string): string | null {
    const row = this.stateDb.get<{ selected_thread_id: string | null }>(
      "SELECT selected_thread_id FROM thread_selection WHERE stream_id = ? LIMIT 1",
      streamId,
    );
    return row?.selected_thread_id ?? null;
  }

  private insertThread(thread: Thread): void {
    this.stateDb.run(
      `INSERT INTO threads (
        id, stream_id, title, status, sort_index, pane_target, resume_session_id,
        created_at, updated_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      thread.id,
      thread.stream_id,
      thread.title,
      thread.status,
      thread.sort_index,
      thread.pane_target,
      thread.resume_session_id,
      thread.created_at,
      thread.updated_at,
    );
  }

  private setSelected(streamId: string, threadId: string): void {
    this.stateDb.run(
      `INSERT INTO thread_selection (stream_id, selected_thread_id)
       VALUES (?, ?)
       ON CONFLICT(stream_id) DO UPDATE SET selected_thread_id = excluded.selected_thread_id`,
      streamId,
      threadId,
    );
  }

  private ensureThreadExists(streamId: string, threadId: string): void {
    const row = this.stateDb.get<{ id: string }>(
      "SELECT id FROM threads WHERE stream_id = ? AND id = ? LIMIT 1",
      streamId,
      threadId,
    );
    if (!row) throw new Error(`unknown thread: ${threadId}`);
  }

  private migrateLegacyIfNeeded(streamId: string): void {
    const existing = this.stateDb.get<{ c: number }>("SELECT COUNT(*) AS c FROM threads WHERE stream_id = ?", streamId);
    if ((existing?.c ?? 0) > 0) return;
    const path = join(this.legacyDir, `${streamId}.json`);
    if (!existsSync(path)) return;
    try {
      const legacy = normalizeState(JSON.parse(readFileSync(path, "utf8")) as PersistedThreadState, streamId);
      this.stateDb.transaction(() => {
        for (const thread of legacy.threads) this.insertThread(thread);
        if (legacy.selectedThreadId) this.setSelected(streamId, legacy.selectedThreadId);
      });
      this.logger?.info("imported legacy thread state into sqlite", {
        streamId,
        count: legacy.threads.length,
      });
    } catch (error) {
      this.logger?.warn("failed to import legacy thread state", {
        streamId,
        error: errorMessage(error),
      });
    }
  }
}

function rowToThread(row: Record<string, unknown>): Thread {
  return {
    id: String(row.id ?? ""),
    stream_id: String(row.stream_id ?? ""),
    title: String(row.title ?? ""),
    status: String(row.status ?? "queued") as ThreadStatus,
    sort_index: Number(row.sort_index ?? 0),
    created_at: String(row.created_at ?? new Date(0).toISOString()),
    updated_at: String(row.updated_at ?? row.created_at ?? new Date(0).toISOString()),
    pane_target: String(row.pane_target ?? ""),
    resume_session_id: String(row.resume_session_id ?? ""),
    custom_prompt: row.custom_prompt != null ? String(row.custom_prompt) : null,
    closed_at: row.closed_at != null ? String(row.closed_at) : null,
  };
}

function normalizeState(state: PersistedThreadState, streamId: string): PersistedThreadState {
  const threads = (Array.isArray(state.threads) ? state.threads : [])
    .map((thread, index) => ({
      ...thread,
      stream_id: thread.stream_id || streamId,
      sort_index: Number.isFinite(thread.sort_index) ? thread.sort_index : index,
      resume_session_id: thread.resume_session_id ?? "",
    }))
    .sort((a, b) => a.sort_index - b.sort_index || a.created_at.localeCompare(b.created_at))
    .map((thread, index) => ({ ...thread, sort_index: index }));
  return {
    selectedThreadId: state.selectedThreadId && threads.some((thread) => thread.id === state.selectedThreadId)
      ? state.selectedThreadId
      : threads[0]?.id ?? null,
    threads,
  };
}

function paneSessionName(stream: Stream) {
  return stream.panes.working.split(":")[0];
}

function createThreadId() {
  return createId("b");
}

function createWindowName() {
  return randomBytes(3).toString("hex");
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
