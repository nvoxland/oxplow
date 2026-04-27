/**
 * Toast queue for fire-and-undo destructive actions. Replaces blocking
 * confirm modals for non-row-anchored destructives (complete thread,
 * archive, etc.) — the action fires immediately, a toast pops up, and
 * the user has ~7s to undo.
 *
 * The store is a tiny in-memory pub/sub. UI subscribes via React's
 * useSyncExternalStore (see `useToasts.ts`). Auto-dismiss timers live
 * on the UI side so test code can drive state without a real clock.
 */

export interface ToastInput {
  message: string;
  /** Optional undo callback. If omitted the toast still appears but only "Dismiss" is offered. */
  onUndo?: () => void;
  /** Optional override for the action label. Default: "Undo". */
  actionLabel?: string;
}

export interface Toast {
  id: string;
  message: string;
  onUndo?: () => void;
  actionLabel: string;
  createdAt: number;
}

export interface ToastStore {
  getSnapshot(): readonly Toast[];
  subscribe(listener: () => void): () => void;
  push(input: ToastInput): string;
  dismiss(id: string): void;
  undo(id: string): void;
}

let nextSeq = 1;
function makeId(): string {
  return `t-${Date.now().toString(36)}-${(nextSeq++).toString(36)}`;
}

export function createToastStore(): ToastStore {
  let toasts: Toast[] = [];
  const listeners = new Set<() => void>();

  function emit() {
    for (const fn of listeners) fn();
  }

  return {
    getSnapshot() {
      return toasts;
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    push(input) {
      const id = makeId();
      const toast: Toast = {
        id,
        message: input.message,
        onUndo: input.onUndo,
        actionLabel: input.actionLabel ?? "Undo",
        createdAt: Date.now(),
      };
      toasts = [...toasts, toast];
      emit();
      return id;
    },
    dismiss(id) {
      const next = toasts.filter((t) => t.id !== id);
      if (next.length === toasts.length) return;
      toasts = next;
      emit();
    },
    undo(id) {
      const toast = toasts.find((t) => t.id === id);
      if (!toast) return;
      toasts = toasts.filter((t) => t.id !== id);
      emit();
      toast.onUndo?.();
    },
  };
}

let singleton: ToastStore | null = null;

/** Process-wide toast store. */
export function getToastStore(): ToastStore {
  if (!singleton) singleton = createToastStore();
  return singleton;
}

/** Convenience: push into the singleton. */
export function showToast(input: ToastInput): string {
  return getToastStore().push(input);
}
