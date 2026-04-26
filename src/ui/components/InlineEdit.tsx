import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";

/**
 * Click-to-edit field. Replaces PromptDialog modals for edit-X-in-place
 * flows.
 *
 * Click the displayed value to swap to an input; Enter commits via
 * `onCommit`; Escape reverts; blur commits unless Escape was pressed
 * (the cancel-latch must be a ref because React state updates are
 * async and the blur fires synchronously). See WorkItemDetail's
 * EditableField for the older inline-only version of this pattern.
 *
 * Variants:
 * - Pass `multiline` for textarea behavior (Cmd/Ctrl+Enter commits;
 *   Enter inserts newline).
 * - Pass `allowEmpty` to permit clearing the field (otherwise empty
 *   reverts).
 * - Pass `placeholder` to render a muted placeholder when the value is
 *   empty in display mode.
 * - Pass `renderDisplay` to fully control the display element (e.g.
 *   when the value is rendered as a label inside a chip).
 */
export function InlineEdit({
  value,
  onCommit,
  placeholder = "(empty)",
  multiline = false,
  allowEmpty = false,
  displayStyle,
  inputStyle,
  testId,
  ariaLabel,
  renderDisplay,
  startEditing = false,
  onCancel,
}: {
  value: string;
  onCommit(next: string): void;
  placeholder?: string;
  multiline?: boolean;
  allowEmpty?: boolean;
  displayStyle?: CSSProperties;
  inputStyle?: CSSProperties;
  testId?: string;
  ariaLabel?: string;
  renderDisplay?(value: string, beginEdit: () => void): React.ReactNode;
  /** When true, mount in editing state (used for "create new …" rows). */
  startEditing?: boolean;
  /** Called when the user explicitly cancels via Escape; useful for `startEditing` mode to dismiss the row. */
  onCancel?(): void;
}) {
  const [editing, setEditing] = useState(startEditing);
  const [draft, setDraft] = useState(value);
  const inputRef = useRef<HTMLInputElement | HTMLTextAreaElement | null>(null);
  const cancelLatch = useRef(false);

  useEffect(() => {
    if (!editing) setDraft(value);
  }, [editing, value]);

  useEffect(() => {
    if (editing) {
      const node = inputRef.current;
      node?.focus();
      if (node && "select" in node) node.select();
    }
  }, [editing]);

  function commit(next: string) {
    const trimmed = multiline ? next : next.trim();
    if (!allowEmpty && trimmed.length === 0) {
      setEditing(false);
      onCancel?.();
      return;
    }
    setEditing(false);
    if (trimmed !== value) onCommit(trimmed);
  }

  function cancel() {
    cancelLatch.current = true;
    setDraft(value);
    setEditing(false);
    onCancel?.();
    inputRef.current?.blur();
  }

  function beginEdit() {
    cancelLatch.current = false;
    setDraft(value);
    setEditing(true);
  }

  if (!editing) {
    if (renderDisplay) {
      return <>{renderDisplay(value, beginEdit)}</>;
    }
    const showPlaceholder = value.trim().length === 0;
    return (
      <span
        role="button"
        tabIndex={0}
        onClick={beginEdit}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            beginEdit();
          }
        }}
        data-testid={testId}
        aria-label={ariaLabel}
        style={{ ...defaultDisplayStyle, ...(displayStyle ?? {}), color: showPlaceholder ? "var(--muted)" : undefined }}
      >
        {showPlaceholder ? placeholder : value}
      </span>
    );
  }

  if (multiline) {
    return (
      <textarea
        ref={(el) => { inputRef.current = el; }}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            cancel();
          } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            commit(draft);
          }
        }}
        onBlur={() => {
          if (cancelLatch.current) { cancelLatch.current = false; return; }
          commit(draft);
        }}
        data-testid={testId}
        aria-label={ariaLabel}
        style={{ ...defaultInputStyle, minHeight: 60, ...(inputStyle ?? {}) }}
      />
    );
  }
  return (
    <input
      ref={(el) => { inputRef.current = el; }}
      type="text"
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          cancel();
        } else if (e.key === "Enter") {
          e.preventDefault();
          commit(draft);
        }
      }}
      onBlur={() => {
        if (cancelLatch.current) { cancelLatch.current = false; return; }
        commit(draft);
      }}
      data-testid={testId}
      aria-label={ariaLabel}
      style={{ ...defaultInputStyle, ...(inputStyle ?? {}) }}
    />
  );
}

const defaultDisplayStyle: CSSProperties = {
  cursor: "text",
  display: "inline-block",
  padding: "2px 4px",
  borderRadius: 3,
  outline: "none",
};

const defaultInputStyle: CSSProperties = {
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "4px 6px",
  fontFamily: "inherit",
  fontSize: 12,
  width: "100%",
  boxSizing: "border-box",
};
