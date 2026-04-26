import type { CSSProperties } from "react";
import { useEffect, useSyncExternalStore } from "react";
import { getToastStore, type Toast } from "./toastStore.js";

const AUTO_DISMISS_MS = 7000;

/**
 * Bottom-of-window toast stack. Renders the live toast list from the
 * singleton store. Each toast auto-dismisses after ~7s; clicking the
 * action button calls `onUndo` (if any) and removes the toast.
 *
 * Mount once near the app root.
 */
export function UndoToastStack() {
  const store = getToastStore();
  const toasts = useSyncExternalStore(
    store.subscribe,
    () => store.getSnapshot(),
    () => store.getSnapshot(),
  );
  if (toasts.length === 0) return null;
  return (
    <div style={stackStyle} data-testid="undo-toast-stack">
      {toasts.map((toast) => (
        <ToastRow key={toast.id} toast={toast} />
      ))}
    </div>
  );
}

function ToastRow({ toast }: { toast: Toast }) {
  const store = getToastStore();
  useEffect(() => {
    const timer = setTimeout(() => {
      store.dismiss(toast.id);
    }, AUTO_DISMISS_MS);
    return () => clearTimeout(timer);
  }, [store, toast.id]);
  return (
    <div style={toastStyle} data-testid={`undo-toast-${toast.id}`}>
      <span style={messageStyle}>{toast.message}</span>
      {toast.onUndo ? (
        <button
          type="button"
          onClick={() => store.undo(toast.id)}
          data-testid={`undo-toast-action-${toast.id}`}
          style={actionStyle}
        >
          {toast.actionLabel}
        </button>
      ) : null}
      <button
        type="button"
        onClick={() => store.dismiss(toast.id)}
        aria-label="Dismiss"
        data-testid={`undo-toast-dismiss-${toast.id}`}
        style={dismissStyle}
      >
        ×
      </button>
    </div>
  );
}

const stackStyle: CSSProperties = {
  position: "fixed",
  bottom: 24,
  right: 24,
  display: "flex",
  flexDirection: "column",
  gap: 8,
  zIndex: 6000,
  pointerEvents: "none",
};

const toastStyle: CSSProperties = {
  pointerEvents: "auto",
  display: "flex",
  alignItems: "center",
  gap: 12,
  background: "var(--bg-1)",
  color: "var(--fg)",
  border: "1px solid var(--border-strong)",
  borderRadius: 6,
  padding: "8px 12px",
  fontSize: 12,
  minWidth: 240,
  maxWidth: 380,
  boxShadow: "0 6px 20px rgba(0, 0, 0, 0.45)",
};

const messageStyle: CSSProperties = {
  flex: 1,
  whiteSpace: "pre-wrap",
};

const actionStyle: CSSProperties = {
  background: "transparent",
  border: "1px solid var(--border)",
  color: "var(--accent)",
  borderRadius: 4,
  padding: "3px 8px",
  fontFamily: "inherit",
  fontSize: 11,
  cursor: "pointer",
};

const dismissStyle: CSSProperties = {
  background: "transparent",
  border: "none",
  color: "var(--muted)",
  fontSize: 16,
  lineHeight: 1,
  padding: "0 2px",
  cursor: "pointer",
};
