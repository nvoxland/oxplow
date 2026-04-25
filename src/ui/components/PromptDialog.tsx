import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";

/**
 * Themed single-input prompt. Replaces `window.prompt()`, which Electron
 * disables (returns null silently). Submit on Enter, cancel on Escape or
 * backdrop click; the input is autofocused and selected on open.
 */
export function PromptDialog({
  message,
  initialValue = "",
  placeholder,
  confirmLabel = "OK",
  cancelLabel = "Cancel",
  allowEmpty = false,
  onSubmit,
  onCancel,
}: {
  message: string;
  initialValue?: string;
  placeholder?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  allowEmpty?: boolean;
  onSubmit(value: string): void;
  onCancel(): void;
}) {
  const [value, setValue] = useState(initialValue);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    inputRef.current?.select();
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

  const trimmed = value.trim();
  const canSubmit = allowEmpty || trimmed.length > 0;

  return (
    <div
      role="dialog"
      aria-modal="true"
      data-testid="prompt-dialog"
      style={backdropStyle}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onCancel(); }}
    >
      <form
        style={panelStyle}
        onSubmit={(e) => {
          e.preventDefault();
          if (!canSubmit) return;
          onSubmit(allowEmpty ? value : trimmed);
        }}
      >
        <div style={messageStyle}>{message}</div>
        <input
          ref={inputRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder={placeholder}
          data-testid="prompt-dialog-input"
          style={inputStyle}
          autoFocus
        />
        <div style={buttonRowStyle}>
          <button type="button" onClick={onCancel} style={secondaryButtonStyle} data-testid="prompt-dialog-cancel">
            {cancelLabel}
          </button>
          <button type="submit" disabled={!canSubmit} style={primaryButtonStyle} data-testid="prompt-dialog-submit">
            {confirmLabel}
          </button>
        </div>
      </form>
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
  minWidth: 360,
  maxWidth: 520,
  background: "var(--bg-1)",
  border: "1px solid var(--border-strong)",
  borderRadius: 8,
  padding: 16,
  boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 16px 40px rgba(0, 0, 0, 0.5)",
  display: "flex",
  flexDirection: "column",
  gap: 12,
};

const messageStyle: CSSProperties = {
  color: "var(--fg)",
  fontSize: 13,
  lineHeight: 1.45,
};

const inputStyle: CSSProperties = {
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "6px 8px",
  fontFamily: "inherit",
  fontSize: 13,
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
