import type { CSSProperties } from "react";
import { useEffect, useRef } from "react";

/**
 * Themed destructive-action confirmation. Replaces `window.confirm()` so
 * every destructive prompt in oxplow uses the same visual language —
 * `window.confirm()` is unstyled, non-dismissable by outside click, and
 * locks rendering of the whole window.
 *
 * Usage: render conditionally (`{confirmState ? <ConfirmDialog .../> :
 * null}`). Confirm autofocuses the cancel button so pressing Enter on an
 * accidental Cmd-Delete hotkey does NOT immediately destroy anything.
 */
export function ConfirmDialog({
  message,
  confirmLabel = "Delete",
  cancelLabel = "Cancel",
  destructive = true,
  onConfirm,
  onCancel,
}: {
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  destructive?: boolean;
  onConfirm(): void;
  onCancel(): void;
}) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    cancelRef.current?.focus();
  }, []);
  useEffect(() => {
    function handler(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        onCancel();
      }
    }
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onCancel]);

  return (
    <div
      role="dialog"
      aria-modal="true"
      data-testid="confirm-dialog"
      style={backdropStyle}
    >
      <div style={panelStyle}>
        <div style={messageStyle}>{message}</div>
        <div style={buttonRowStyle}>
          <button
            type="button"
            ref={cancelRef}
            onClick={onCancel}
            data-testid="confirm-dialog-cancel"
            style={secondaryButtonStyle}
          >
            {cancelLabel}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            data-testid="confirm-dialog-confirm"
            style={destructive ? destructiveButtonStyle : primaryButtonStyle}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

const backdropStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0, 0, 0, 0.45)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 5000,
};

const panelStyle: CSSProperties = {
  minWidth: 320,
  maxWidth: 480,
  background: "var(--bg-1)",
  border: "1px solid var(--border-strong)",
  borderRadius: 8,
  padding: 16,
  boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 16px 40px rgba(0, 0, 0, 0.5)",
  display: "flex",
  flexDirection: "column",
  gap: 14,
};

const messageStyle: CSSProperties = {
  color: "var(--fg)",
  fontSize: 13,
  lineHeight: 1.45,
  whiteSpace: "pre-wrap",
};

const buttonRowStyle: CSSProperties = {
  display: "flex",
  justifyContent: "flex-end",
  gap: 8,
};

const baseButtonStyle: CSSProperties = {
  border: "1px solid var(--border)",
  borderRadius: 6,
  padding: "6px 12px",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 12,
};

const secondaryButtonStyle: CSSProperties = {
  ...baseButtonStyle,
  background: "var(--bg-2)",
  color: "var(--fg)",
};

const primaryButtonStyle: CSSProperties = {
  ...baseButtonStyle,
  background: "var(--accent)",
  color: "#fff",
  borderColor: "var(--accent)",
};

const destructiveButtonStyle: CSSProperties = {
  ...baseButtonStyle,
  background: "#b91c1c",
  color: "#fff",
  borderColor: "#b91c1c",
};
