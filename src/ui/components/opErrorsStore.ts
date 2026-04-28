/**
 * In-memory error log for failed async operations (git merge/push/pull,
 * commit, note save, snapshot restore, etc.). Replaces the modal
 * `window.alert` pattern: failures push a structured record into this
 * store, the RailHud surfaces them as red rows, and clicking a row
 * opens a dedicated page with the full output. Capped at the last
 * MAX_ENTRIES; nothing persists across reload.
 */

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
}

export interface OpError extends Required<Omit<OpErrorInput, "exitCode" | "threadId">> {
  id: string;
  exitCode: number | null;
  threadId: string | null;
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
}

let nextSeq = 1;
function makeId(): string {
  return `oe-${Date.now().toString(36)}-${(nextSeq++).toString(36)}`;
}

export function createOpErrorsStore(): OpErrorsStore {
  let entries: OpError[] = [];
  let activeThreadId: string | null = null;
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
        at: Date.now(),
        seen: false,
      };
      const next = [entry, ...entries];
      entries = next.length > MAX_ENTRIES ? next.slice(0, MAX_ENTRIES) : next;
      emit();
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
  };
}

let singleton: OpErrorsStore | null = null;

/** Process-wide op-errors store. */
export function getOpErrorsStore(): OpErrorsStore {
  if (!singleton) singleton = createOpErrorsStore();
  return singleton;
}

/** Convenience: push into the singleton, returning the new id. */
export function recordOpError(input: OpErrorInput): string {
  return getOpErrorsStore().push(input);
}
