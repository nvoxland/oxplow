import React, { useEffect, useRef, useState } from "react";

/**
 * The Assistant's prompt box. Enter sends, Shift+Enter adds a newline,
 * Escape stops a running turn (and is not passed on, or the dock panel would
 * close). A running turn leaves the box read-only, not disabled, so Escape
 * still reaches it. The draft is this box's own state: a keystroke re-renders
 * the box and nothing above it. Suggestions fill the draft; they never send.
 */
export function AssistantPromptBox({
  busy,
  disabled,
  suggestions,
  draft: pushed = null,
  onSend,
  onCancel,
}: {
  /** A turn is in flight: Enter does nothing (prompts are never queued). */
  busy: boolean;
  /** Nothing to talk to (no model, no session). */
  disabled: boolean;
  suggestions: string[];
  /** Text to put in the box (changes with `n`); never sent. */
  draft?: { text: string; n: number } | null;
  onSend(text: string): Promise<void> | void;
  onCancel(): void;
}) {
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  useEffect(() => {
    if (pushed) {
      setDraft(pushed.text);
      inputRef.current?.focus();
    }
  }, [pushed?.n]);
  const canSend = !busy && !disabled && draft.trim().length > 0;

  const send = async () => {
    if (!canSend) return;
    const text = draft;
    setDraft("");
    await onSend(text);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void send();
    } else if (e.key === "Escape" && busy) {
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    }
  };

  return (
    <div data-testid="assistant-prompt" style={{ borderTop: "1px solid var(--border-subtle)", background: "var(--surface-card)" }}>
      {suggestions.length > 0 && !busy ? (
        <div data-testid="assistant-suggestions" style={{ display: "flex", gap: 6, flexWrap: "wrap", padding: "6px 8px 0" }}>
          {suggestions.map((s) => (
            <button
              key={s}
              type="button"
              data-testid="assistant-suggestion"
              title="Put this question in the box"
              onClick={() => {
                setDraft(s);
                inputRef.current?.focus();
              }}
              style={chip}
            >
              {s}
            </button>
          ))}
        </div>
      ) : null}
      <div style={{ display: "flex", gap: 8, alignItems: "flex-end", padding: 8 }}>
        <textarea
          ref={inputRef}
          data-testid="assistant-input"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={onKeyDown}
          readOnly={busy}
          disabled={disabled}
          rows={2}
          placeholder={
            disabled
              ? "The Assistant has no model"
              : busy
                ? "Working — Escape stops it"
                : "Ask — Enter sends, Shift+Enter for a new line"
          }
          style={{
            flex: 1,
            resize: "vertical",
            minHeight: 40,
            background: "var(--surface-app)",
            color: "var(--text-primary)",
            border: "1px solid var(--border-strong)",
            borderRadius: 4,
            padding: 6,
            font: "inherit",
          }}
        />
        {busy ? (
          <button type="button" data-testid="assistant-stop" onClick={onCancel} style={button(false)}>
            Stop
          </button>
        ) : (
          <button type="button" data-testid="assistant-send" onClick={() => void send()} disabled={!canSend} style={button(true)}>
            Send
          </button>
        )}
      </div>
    </div>
  );
}

const chip: React.CSSProperties = {
  background: "var(--surface-app)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 12,
  padding: "2px 10px",
  fontSize: 12,
  cursor: "pointer",
};

function button(primary: boolean): React.CSSProperties {
  return {
    background: primary ? "var(--button-primary-bg)" : "transparent",
    color: primary ? "var(--button-primary-fg)" : "var(--text-primary)",
    border: primary ? "none" : "1px solid var(--border-strong)",
    borderRadius: 4,
    padding: "6px 14px",
    cursor: "pointer",
  };
}
