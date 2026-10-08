/**
 * In-memory error log for failed async operations (git merge/push/pull,
 * commit, note save, snapshot restore, etc.). Replaces the modal
 * `window.alert` pattern: failures push a structured record into this
 * store, the RailHud surfaces them as red rows, and clicking a row
 * opens a dedicated page with the full output. Capped at the last
 * MAX_ENTRIES; nothing persists across reload here. Each one is also
 * reported to the daemon (`oxplow.ui.report_error`, tsk1072), so the agent can
 * read what the person saw in `v_op_error`.
 */

import { runCommand } from "../api.js";
import { logUi, type UiLogLevel } from "../logger.js";
import { streamRef, threadRef } from "../recordRefs.js";

const MAX_ENTRIES = 20;

export interface OpErrorInput {
  /** Short user-facing label, e.g. "Merge bugfixes into current". */
  label: string;
  /** Optional shell-style command preview, e.g. "git merge bugfixes". */
  command?: string;
  /** Captured stderr (preferred) — typically the most useful field. */
  stderr?: string;
  /** Captured stdout, if anything was emitted before failure. */
  stdout?: string;
  /** Numeric exit code, if known. */
  exitCode?: number | null;
  /** Free-form long message when no stderr is available (thrown Error etc.). */
  message?: string;
  /** Thread the operation was started from. When omitted, defaults to
   *  the store's active-thread context (set via setActiveThread). null
   *  is the explicit "no thread / stream-wide" sentinel. */
  threadId?: string | null;
  /** Argv passed to the underlying child process (without `git -C <dir>`). */
  args?: string[];
  /** Wall-clock duration in ms, if measured. */
  durationMs?: number;
  /** Signal name if the child was killed by signal (SIGKILL, SIGTERM, …). */
  signal?: string | null;
  /** True when the runner caught a failure but stderr/stdout/exitCode were
   *  all empty — diagnostic flag for the "blank op-error" race. */
  blankFailure?: boolean;
}

export interface OpError extends Required<Omit<OpErrorInput, "exitCode" | "threadId" | "args" | "durationMs" | "signal" | "blankFailure">> {
  id: string;
  exitCode: number | null;
  threadId: string | null;
  /** The stream on screen when it happened (`str1`), if any. */
  streamId: string | null;
  args: string[] | null;
  durationMs: number | null;
  signal: string | null;
  blankFailure: boolean;
  at: number;
  /** Has the user opened the page for this error? Used to gate the
   *  RailHud "unread" dot. */
  seen: boolean;
}

export interface OpErrorsStore {
  getSnapshot(): readonly OpError[];
  subscribe(listener: () => void): () => void;
  push(input: OpErrorInput): string;
  markSeen(id: string): void;
  dismiss(id: string): void;
  clear(): void;
  get(id: string): OpError | null;
  /** Set the thread that newly-pushed errors are attributed to when
   *  the caller doesn't pass an explicit threadId. App.tsx wires this
   *  to the currently-selected thread. */
  setActiveThread(threadId: string | null): void;
  /** The stream on screen: stamped on each entry, so one from no thread
   *  still names where it happened (tsk1079). */
  setActiveStream(streamId: string | null): void;
}

/** Called with each entry as it's pushed. */
export type OpErrorReporter = (entry: OpError) => void;

/**
 * A reporter that records each op error on the daemon as
 * `oxplow.ui.report_error`, without waiting for it. A report that fails is
 * logged, never pushed as another op error — that would report itself
 * again, without end.
 */
export function reportOpErrorTo(
  run: (name: string, input: unknown) => Promise<unknown>,
  log: (level: UiLogLevel, message: string, context?: Record<string, unknown>) => void,
): OpErrorReporter {
  return (entry) => {
    const input: Record<string, unknown> = { label: entry.label };
    if (entry.command) input.command = entry.command;
    if (entry.message) input.message = entry.message;
    if (entry.stderr) input.stderr = entry.stderr;
    if (entry.stdout) input.stdout = entry.stdout;
    if (entry.exitCode !== null) input.exit_code = entry.exitCode;
    // A thread names its own stream; from no thread, the one on screen
    // does, so its agent can read the output (tsk1079).
    if (entry.threadId !== null) input.thread = threadRef(entry.threadId);
    else if (entry.streamId !== null) input.stream = streamRef(entry.streamId);
    if (entry.signal !== null) input.signal = entry.signal;
    if (entry.durationMs !== null) input.duration_ms = Math.round(entry.durationMs);
    run("oxplow.ui.report_error", input).catch((error: unknown) => {
      log("warn", "couldn't report an op error to the daemon", {
        label: entry.label,
        error: error instanceof Error ? error.message : String(error),
      });
    });
  };
}

let nextSeq = 1;
function makeId(): string {
  return `oe-${Date.now().toString(36)}-${(nextSeq++).toString(36)}`;
}

export function createOpErrorsStore(report?: OpErrorReporter): OpErrorsStore {
  let entries: OpError[] = [];
  let activeThreadId: string | null = null;
  let activeStreamId: string | null = null;
  const listeners = new Set<() => void>();

  function emit() {
    for (const fn of listeners) fn();
  }

  return {
    getSnapshot() {
      return entries;
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    push(input) {
      const id = makeId();
      const entry: OpError = {
        id,
        label: input.label,
        command: input.command ?? "",
        stderr: input.stderr ?? "",
        stdout: input.stdout ?? "",
        message: input.message ?? "",
        exitCode: input.exitCode ?? null,
        threadId: input.threadId !== undefined ? input.threadId : activeThreadId,
        streamId: activeStreamId,
        args: input.args ?? null,
        durationMs: input.durationMs ?? null,
        signal: input.signal ?? null,
        blankFailure: input.blankFailure ?? false,
        at: Date.now(),
        seen: false,
      };
      const next = [entry, ...entries];
      entries = next.length > MAX_ENTRIES ? next.slice(0, MAX_ENTRIES) : next;
      emit();
      report?.(entry);
      return id;
    },
    markSeen(id) {
      let changed = false;
      entries = entries.map((e) => {
        if (e.id === id && !e.seen) {
          changed = true;
          return { ...e, seen: true };
        }
        return e;
      });
      if (changed) emit();
    },
    dismiss(id) {
      const next = entries.filter((e) => e.id !== id);
      if (next.length === entries.length) return;
      entries = next;
      emit();
    },
    clear() {
      if (entries.length === 0) return;
      entries = [];
      emit();
    },
    get(id) {
      return entries.find((e) => e.id === id) ?? null;
    },
    setActiveThread(threadId) {
      activeThreadId = threadId;
    },
    setActiveStream(streamId) {
      activeStreamId = streamId;
    },
  };
}

let singleton: OpErrorsStore | null = null;

/** Process-wide op-errors store. */
export function getOpErrorsStore(): OpErrorsStore {
  if (!singleton) singleton = createOpErrorsStore(reportOpErrorTo(runCommand, logUi));
  return singleton;
}

/** Convenience: push into the singleton, returning the new id. */
export function recordOpError(input: OpErrorInput): string {
  return getOpErrorsStore().push(input);
}
