import type { CSSProperties, ReactNode } from "react";
import { useEffect, useRef, useState } from "react";

/**
 * Inline destructive-confirm pattern. Replaces ConfirmDialog modals for
 * destructive actions that live on a row or button.
 *
 * First click on the trigger swaps to a `[Confirm] [Cancel]` pair in the
 * same horizontal real-estate. Pressing Enter on the auto-focused
 * Confirm button (or clicking it) fires `onConfirm`. Escape, a press
 * outside the pair, focus moving elsewhere, or the Cancel button reverts
 * to the trigger. Focus moving to an element around the pair doesn't:
 * WebKit (the desktop window's engine) doesn't focus a button that's
 * clicked, so a click on Confirm first moves focus to the nearest
 * focusable ancestor — a tab, a row — and reverting then would swallow
 * the click.
 *
 * Use this when the action lives on a specific UI element (delete row,
 * delete note, restore file). For non-row-anchored destructives (archive
 * thread, complete batch), use the toast queue instead — fire the
 * destructive immediately and let the user undo from the toast.
 */
export function InlineConfirm({
  triggerLabel,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  onConfirm,
  triggerStyle,
  testIdPrefix,
  disabled = false,
  title,
  children,
}: {
  triggerLabel?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  onConfirm(): void;
  triggerStyle?: CSSProperties;
  testIdPrefix?: string;
  disabled?: boolean;
  title?: string;
  /** Optional render-prop for a fully custom trigger element (icon button, kebab item, etc.). When provided, `triggerLabel` is ignored. The render prop receives an `arm` callback to enter confirm state. */
  children?: (arm: () => void) => ReactNode;
}) {
  const [armed, setArmed] = useState(false);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const containerRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (armed) {
      confirmRef.current?.focus();
    }
  }, [armed]);

  useEffect(() => {
    if (!armed) return;
    function handleKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        setArmed(false);
      }
    }
    // A press anywhere outside the pair reverts it.
    function handlePointer(event: PointerEvent | MouseEvent) {
      if (!containerRef.current?.contains(event.target as Node)) setArmed(false);
    }
    window.addEventListener("keydown", handleKey);
    document.addEventListener("mousedown", handlePointer);
    return () => {
      window.removeEventListener("keydown", handleKey);
      document.removeEventListener("mousedown", handlePointer);
    };
  }, [armed]);

  function handleBlur(event: React.FocusEvent<HTMLSpanElement>) {
    const next = event.relatedTarget as Node | null;
    // Focus stayed within the pair (Tab to Cancel), or went to an element
    // around it — WebKit's focus for a click on Confirm — or nowhere
    // focusable: stay armed. A press outside reverts it (above).
    if (!next || containerRef.current?.contains(next) || next.contains(containerRef.current)) return;
    setArmed(false);
  }

  if (!armed) {
    if (children) {
      return <>{children(() => { if (!disabled) setArmed(true); })}</>;
    }
    return (
      <button
        type="button"
        title={title}
        disabled={disabled}
        onClick={() => setArmed(true)}
        data-testid={testIdPrefix ? `${testIdPrefix}-trigger` : undefined}
        style={{ ...defaultTriggerStyle, ...(triggerStyle ?? {}) }}
      >
        {triggerLabel ?? "Delete"}
      </button>
    );
  }

  return (
    <span ref={containerRef} style={pairStyle} onBlur={handleBlur}>
      <button
        type="button"
        ref={confirmRef}
        onClick={() => {
          setArmed(false);
          onConfirm();
        }}
        data-testid={testIdPrefix ? `${testIdPrefix}-confirm` : undefined}
        style={destructiveButtonStyle}
      >
        {confirmLabel}
      </button>
      <button
        type="button"
        onClick={() => setArmed(false)}
        data-testid={testIdPrefix ? `${testIdPrefix}-cancel` : undefined}
        style={cancelButtonStyle}
      >
        {cancelLabel}
      </button>
    </span>
  );
}

const defaultTriggerStyle: CSSProperties = {
  border: "1px solid var(--border)",
  background: "var(--bg-2)",
  color: "var(--fg)",
  borderRadius: 4,
  padding: "4px 8px",
  fontSize: 11,
  cursor: "pointer",
};

const pairStyle: CSSProperties = {
  display: "inline-flex",
  gap: 4,
  alignItems: "center",
};

const destructiveButtonStyle: CSSProperties = {
  background: "#b91c1c",
  color: "#fff",
  border: "1px solid #b91c1c",
  borderRadius: 4,
  padding: "4px 8px",
  fontSize: 11,
  cursor: "pointer",
};

const cancelButtonStyle: CSSProperties = {
  background: "var(--bg-2)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "4px 8px",
  fontSize: 11,
  cursor: "pointer",
};
