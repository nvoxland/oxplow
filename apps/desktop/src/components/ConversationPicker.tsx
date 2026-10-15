/**
 * Choose one of an agent session's saved conversations — to resume it in
 * the session, or to fork it into the session — from what its harness
 * lists for the worktree (`.context/terminal.md` "Choosing another
 * conversation"). ↑/↓ move, Enter or a click takes the row, Escape or a
 * press outside cancels.
 */
import { useEffect, useRef, useState } from "react";

import { forkConversation, listNativeConversations, resumeConversation } from "../api.js";
import { conversationRows, type ConversationRow, type PickerMode } from "./conversationRows.js";
import { usePopoverDismiss } from "./usePopoverDismiss.js";

interface Props {
  sessionId: string;
  mode: PickerMode;
  /** The session took the conversation: its next launch runs it. */
  onChosen: () => void;
  onCancel: () => void;
}

export function ConversationPicker({ sessionId, mode, onChosen, onCancel }: Props) {
  const [rows, setRows] = useState<ConversationRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [cursor, setCursor] = useState(0);
  const [busy, setBusy] = useState(false);
  const boxRef = usePopoverDismiss<HTMLDivElement>(true, onCancel);
  const listRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let live = true;
    listNativeConversations(sessionId)
      .then((listed) => {
        if (!live) return;
        const next = conversationRows(listed, mode);
        setRows(next);
        setCursor(Math.max(0, next.findIndex((r) => r.takeable)));
      })
      .catch((e: unknown) => live && setError(e instanceof Error ? e.message : String(e)));
    return () => {
      live = false;
    };
  }, [sessionId, mode]);

  useEffect(() => listRef.current?.focus(), [rows]);

  const take = async (row: ConversationRow | undefined) => {
    if (!row?.takeable || busy) return;
    setBusy(true);
    setError(null);
    try {
      if (mode === "resume") await resumeConversation(sessionId, row.id);
      else await forkConversation(sessionId, row.id);
      onChosen();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!rows?.length) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setCursor((c) => Math.min(rows.length - 1, c + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setCursor((c) => Math.max(0, c - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      void take(rows[cursor]);
    }
  };

  const title = mode === "resume" ? "Resume a saved conversation" : "Fork a saved conversation";
  return (
    <div
      ref={boxRef}
      data-testid="conversation-picker"
      role="dialog"
      aria-label={title}
      style={{
        position: "absolute",
        top: 24,
        left: "50%",
        transform: "translateX(-50%)",
        width: "min(560px, calc(100% - 32px))",
        maxHeight: "70%",
        display: "flex",
        flexDirection: "column",
        background: "var(--surface-card)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 6,
        boxShadow: "0 8px 24px rgba(0, 0, 0, 0.25)",
        zIndex: 6,
        color: "var(--text-primary)",
        fontSize: 13,
      }}
    >
      <div style={{ padding: "8px 12px", borderBottom: "1px solid var(--border-subtle)", fontWeight: 600 }}>{title}</div>
      {error ? (
        <div data-testid="conversation-picker-error" style={{ padding: "8px 12px", color: "var(--text-danger, #d9534f)" }}>
          {error}
        </div>
      ) : null}
      {rows === null && !error ? <div style={{ padding: "8px 12px", color: "var(--muted)" }}>Reading its saved conversations…</div> : null}
      {rows?.length === 0 ? <div style={{ padding: "8px 12px", color: "var(--muted)" }}>No saved conversations for this worktree.</div> : null}
      {rows?.length ? (
        <div ref={listRef} tabIndex={0} role="listbox" onKeyDown={onKeyDown} style={{ overflowY: "auto", outline: "none" }}>
          {rows.map((row, i) => (
            <button
              key={row.id}
              type="button"
              role="option"
              aria-selected={i === cursor}
              data-testid={`conversation-row-${row.id}`}
              disabled={!row.takeable || busy}
              onMouseEnter={() => setCursor(i)}
              onClick={() => void take(row)}
              style={{
                display: "flex",
                width: "100%",
                gap: 10,
                alignItems: "baseline",
                padding: "6px 12px",
                border: "none",
                textAlign: "left",
                background: i === cursor ? "rgba(74, 158, 255, 0.18)" : "transparent",
                color: row.takeable ? "var(--text-primary)" : "var(--muted)",
                cursor: row.takeable ? "pointer" : "default",
              }}
            >
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{row.label}</span>
              {row.note ? <span style={{ fontSize: 11, color: "var(--muted)" }}>{row.note}</span> : null}
              <span style={{ fontSize: 11, color: "var(--muted)" }}>{[row.when, row.size].filter(Boolean).join(" · ")}</span>
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
