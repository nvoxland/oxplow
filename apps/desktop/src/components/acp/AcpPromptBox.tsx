import React, { useEffect, useRef, useState } from "react";

import { acpPrompt } from "../../api.js";
import { subscribeAgentInput } from "../../agent-input-bus.js";

interface Props {
  /** The ACP agent session (`ses3`) it prompts. */
  sessionId: string;
  /** A turn is in flight: Enter does nothing (prompts are never queued). */
  busy: boolean;
  /** No live session (starting, stopped, failed to open). */
  disabled: boolean;
  visible: boolean;
  onCancel(): void;
  onError(message: string): void;
}

/**
 * The prompt box of an ACP session — the ONLY place that sends an ACP
 * agent a prompt, and only on the person's Enter / Send. Nothing else in
 * the renderer may call `acpPrompt` (no-agent-input-automation.test.ts).
 *
 * Enter sends, Shift+Enter adds a newline, Escape stops a running turn.
 * "Add to agent context" gestures (agent-input-bus) append to the draft
 * while this box is visible; they never send.
 *
 * The draft is this box's own state, so a keystroke re-renders the box
 * and nothing else — never the transcript above it.
 */
export function AcpPromptBox({
  sessionId,
  busy,
  disabled,
  visible,
  onCancel,
  onError,
}: Props) {
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    if (!visible) return;
    return subscribeAgentInput((text) => {
      setDraft((d) => (d && !d.endsWith(" ") && !d.endsWith("\n") ? `${d} ${text}` : `${d}${text}`));
      inputRef.current?.focus();
    });
  }, [visible]);

  const canSend = !busy && !disabled && !sending && draft.trim().length > 0;

  const send = async () => {
    if (!canSend) return;
    setSending(true);
    try {
      await acpPrompt(sessionId, draft);
      setDraft("");
    } catch (err) {
      onError(err instanceof Error ? err.message : String(err));
    } finally {
      setSending(false);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void send();
    } else if (e.key === "Escape" && busy) {
      e.preventDefault();
      onCancel();
    }
  };

  return (
    <div
      data-testid="acp-prompt"
      style={{
        display: "flex",
        gap: 8,
        alignItems: "flex-end",
        padding: 8,
        borderTop: "1px solid var(--border-subtle)",
        background: "var(--surface-card)",
      }}
    >
      <textarea
        ref={inputRef}
        data-testid="acp-prompt-input"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={onKeyDown}
        disabled={disabled}
        rows={3}
        placeholder={
          disabled
            ? "No agent session"
            : busy
              ? "The agent is working — Escape stops it"
              : "Message the agent — Enter sends, Shift+Enter for a new line"
        }
        style={{
          flex: 1,
          resize: "vertical",
          minHeight: 44,
          background: "var(--surface-app)",
          color: "var(--text-primary)",
          border: "1px solid var(--border-strong)",
          borderRadius: 4,
          padding: 6,
          font: "inherit",
        }}
      />
      {busy ? (
        <button
          type="button"
          data-testid="acp-prompt-stop"
          onClick={onCancel}
          style={buttonStyle(false)}
        >
          Stop
        </button>
      ) : (
        <button
          type="button"
          data-testid="acp-prompt-send"
          onClick={() => void send()}
          disabled={!canSend}
          style={buttonStyle(true)}
        >
          {sending ? "Sending…" : "Send"}
        </button>
      )}
    </div>
  );
}

function buttonStyle(primary: boolean): React.CSSProperties {
  return {
    background: primary ? "var(--button-primary-bg)" : "transparent",
    color: primary ? "var(--button-primary-fg)" : "var(--text-primary)",
    border: primary ? "none" : "1px solid var(--border-strong)",
    borderRadius: 4,
    padding: "6px 14px",
    cursor: "pointer",
  };
}
