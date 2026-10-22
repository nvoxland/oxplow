import { useState } from "react";

import type { CommentIntent } from "../../tauri-bridge/generated/bindings.js";
import { Markdown } from "../Markdown/Markdown.js";

/// Shared composer for both the new-comment flow and replies: the one
/// Markdown editor, so a comment is written as it reads. Enter adds a
/// line, Cmd/Ctrl+Enter submits, Escape cancels (per the usability
/// rules). Submit is disabled while empty.
export function CommentComposer({
  submitLabel,
  placeholder,
  autofocus = true,
  showIntent = false,
  intent = "note",
  onIntentChange,
  onSubmit,
  onCancel,
  testIdPrefix,
}: {
  submitLabel: string;
  placeholder?: string;
  autofocus?: boolean;
  showIntent?: boolean;
  intent?: CommentIntent;
  onIntentChange?: (intent: CommentIntent) => void;
  onSubmit: (body: string) => void | Promise<void>;
  onCancel?: () => void;
  testIdPrefix: string;
}) {
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  // A new editor after a send: the next comment starts empty.
  const [round, setRound] = useState(0);

  const canSubmit = body.trim().length > 0 && !busy;

  const submit = async (text = body) => {
    if (text.trim().length === 0 || busy) return;
    setBusy(true);
    try {
      await onSubmit(text.trim());
      setBody("");
      setRound((r) => r + 1);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <Markdown
        key={round}
        value=""
        placeholder={placeholder ?? "Add a comment…"}
        testId={`${testIdPrefix}-input`}
        autofocus={autofocus}
        hidePencil
        onChange={setBody}
        onSubmitShortcut={(text) => void submit(text)}
        onEscape={() => onCancel?.()}
        style={{
          minHeight: 150,
          maxHeight: 320,
          overflowY: "auto",
          padding: "8px 10px",
          background: "var(--surface-card)",
          border: "1px solid var(--border-subtle)",
          borderRadius: 6,
          fontSize: "var(--text-sm)",
        }}
      />
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        {showIntent && (
          <select
            value={intent}
            data-testid={`${testIdPrefix}-intent`}
            onChange={(e) => onIntentChange?.(e.target.value as CommentIntent)}
            style={{
              padding: "4px 6px",
              background: "var(--surface-card)",
              color: "var(--text-secondary)",
              border: "1px solid var(--border-subtle)",
              borderRadius: 6,
              fontSize: "var(--text-xs)",
            }}
          >
            <option value="note">Note to self</option>
            <option value="followup">Wants follow-up</option>
          </select>
        )}
        <div style={{ flex: 1 }} />
        {onCancel && (
          <button
            type="button"
            data-testid={`${testIdPrefix}-cancel`}
            onClick={onCancel}
            style={secondaryBtn}
          >
            Cancel
          </button>
        )}
        <button
          type="button"
          data-testid={`${testIdPrefix}-submit`}
          onClick={() => void submit()}
          disabled={!canSubmit}
          style={{ ...primaryBtn, opacity: canSubmit ? 1 : 0.5 }}
        >
          {submitLabel}
        </button>
      </div>
    </div>
  );
}

const primaryBtn: React.CSSProperties = {
  padding: "4px 12px",
  background: "var(--button-primary-bg)",
  color: "var(--button-primary-fg)",
  border: "none",
  borderRadius: 6,
  fontSize: "var(--text-sm)",
  cursor: "pointer",
};

const secondaryBtn: React.CSSProperties = {
  padding: "4px 10px",
  background: "transparent",
  color: "var(--text-secondary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  fontSize: "var(--text-sm)",
  cursor: "pointer",
};
