import React, { memo, useEffect, useRef } from "react";

import { Markdown } from "../Markdown/Markdown.js";
import type { AssistantItem, AssistantToolCall } from "./assistantApi.js";

/** The conversation: the person's prompts, the Assistant's text, folded
 *  thoughts, tool calls with their status, output and Undo, and permission
 *  cards (Allow / Decline). Follows the end while the person is at the bottom. */
export const AssistantTranscript = memo(function AssistantTranscript({
  items,
  undone,
  onAnswer,
  onUndo,
}: {
  items: AssistantItem[];
  /** Audit ids already undone. */
  undone: ReadonlySet<number>;
  onAnswer(ask: string, allow: boolean): void;
  onUndo(auditId: number): void;
}) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const pinned = useRef(true);
  const last = items[items.length - 1];
  useEffect(() => {
    const el = scrollRef.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [items.length, last?.seq]);

  return (
    <div
      ref={scrollRef}
      data-testid="assistant-transcript"
      onScroll={(e) => {
        const el = e.currentTarget;
        pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
      }}
      style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: "10px 12px", display: "flex", flexDirection: "column", gap: 8 }}
    >
      {items.map((item) => (
        <div key={item.id} data-testid={`assistant-item-${item.id}`} data-kind={item.type}>
          <Item item={item} undone={undone} onAnswer={onAnswer} onUndo={onUndo} />
        </div>
      ))}
    </div>
  );
});

const Item = memo(function Item({
  item,
  undone,
  onAnswer,
  onUndo,
}: {
  item: AssistantItem;
  undone: ReadonlySet<number>;
  onAnswer(ask: string, allow: boolean): void;
  onUndo(auditId: number): void;
}) {
  switch (item.type) {
    case "user":
      return (
        <div style={{ maxWidth: "85%", marginLeft: "auto", ...bubble("var(--accent-soft-bg)") }}>
          <div style={{ whiteSpace: "pre-wrap" }}>{item.text}</div>
        </div>
      );
    case "agent":
      return <Markdown value={item.text} />;
    case "thought":
      return (
        <details style={{ color: "var(--text-secondary)", fontSize: 12 }}>
          <summary>Thinking</summary>
          <div style={{ whiteSpace: "pre-wrap" }}>{item.text}</div>
        </details>
      );
    case "tool":
      return <ToolCard call={item.call} undone={undone} onUndo={onUndo} />;
    case "plan":
      return (
        <div style={bubble("var(--surface-card)")}>
          <div style={{ fontWeight: 600, marginBottom: 4 }}>Plan</div>
          {item.entries.map((e, i) => (
            <div key={i} style={{ color: e.status === "completed" ? "var(--text-muted)" : "var(--text-primary)" }}>
              {e.status === "completed" ? "●" : e.status === "in_progress" ? "◐" : "○"} {e.content}
            </div>
          ))}
        </div>
      );
    case "permission":
      return (
        <div data-testid={`assistant-permission-${item.requestId}`} style={notice("var(--status-waiting)")}>
          <div style={{ marginBottom: 6 }}>
            The Assistant asks to run <strong>{item.title}</strong>
          </div>
          {item.answer ? (
            <div style={{ color: "var(--text-secondary)" }}>{answerLine(item)}</div>
          ) : (
            <div style={{ display: "flex", gap: 8 }}>
              <button type="button" data-testid={`assistant-allow-${item.requestId}`} style={primaryButton} onClick={() => onAnswer(item.requestId, true)}>
                Allow
              </button>
              <button type="button" data-testid={`assistant-decline-${item.requestId}`} style={smallButton} onClick={() => onAnswer(item.requestId, false)}>
                Decline
              </button>
            </div>
          )}
        </div>
      );
    case "policy_denied":
      return (
        <div style={notice("var(--status-waiting)")}>
          oxplow blocked {item.label}: {item.reason}
        </div>
      );
    case "bypass":
      return (
        <div style={notice("var(--severity-high)")}>
          <strong>{item.label} ran without asking.</strong> oxplow would have blocked it: {item.reason}
        </div>
      );
    case "error":
      return (
        <div data-testid="assistant-error-item" style={notice("var(--severity-critical)")}>
          {item.message}
        </div>
      );
  }
});

function answerLine(item: Extract<AssistantItem, { type: "permission" }>): string {
  const a = item.answer;
  if (!a || a.type === "cancelled") return "Cancelled";
  const kind = item.options.find((o) => o.id === a.optionId)?.kind;
  return kind && kind.startsWith("allow") ? "Allowed" : "Declined";
}

function ToolCard({ call, undone, onUndo }: { call: AssistantToolCall; undone: ReadonlySet<number>; onUndo(auditId: number): void }) {
  const output = call.text.join("\n").trim();
  const canUndo = call.auditId != null && call.status === "completed";
  return (
    <div data-testid="assistant-tool" style={{ ...bubble("var(--surface-card)"), fontSize: 12 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "baseline" }}>
        <span style={{ color: "var(--text-secondary)" }}>{call.kind}</span>
        <span style={{ color: "var(--text-primary)", flex: 1, minWidth: 0, overflowWrap: "anywhere" }}>{call.title || call.name || "Tool call"}</span>
        <span data-testid="assistant-tool-status" style={{ color: toolStatusColor(call.status) }}>
          {call.status.replace("_", " ")}
        </span>
        {canUndo ? (
          undone.has(call.auditId!) ? (
            <span style={{ color: "var(--text-muted)" }}>undone</span>
          ) : (
            <button type="button" data-testid="assistant-undo" style={smallButton} onClick={() => onUndo(call.auditId!)}>
              Undo
            </button>
          )
        ) : null}
      </div>
      {output ? (
        <details style={{ marginTop: 4 }}>
          <summary style={{ color: "var(--text-secondary)" }}>Result</summary>
          <pre style={preStyle}>{output}</pre>
        </details>
      ) : null}
    </div>
  );
}

function toolStatusColor(s: AssistantToolCall["status"]): string {
  switch (s) {
    case "completed":
      return "var(--status-done)";
    case "failed":
      return "var(--severity-critical)";
    case "in_progress":
      return "var(--status-running)";
    default:
      return "var(--text-muted)";
  }
}

const bubble = (bg: string): React.CSSProperties => ({ background: bg, border: "1px solid var(--border-subtle)", borderRadius: 6, padding: "6px 10px" });
const notice = (color: string): React.CSSProperties => ({
  borderLeft: `3px solid ${color}`,
  background: "var(--surface-card)",
  padding: "6px 10px",
  borderRadius: 4,
  color: "var(--text-primary)",
  fontSize: 12,
});
const preStyle: React.CSSProperties = { whiteSpace: "pre-wrap", margin: "4px 0 0", maxHeight: 240, overflow: "auto", fontSize: 11, color: "var(--text-secondary)" };
const smallButton: React.CSSProperties = {
  background: "transparent",
  color: "var(--text-primary)",
  border: "1px solid var(--border-strong)",
  borderRadius: 4,
  padding: "2px 10px",
  fontSize: 12,
  cursor: "pointer",
};
const primaryButton: React.CSSProperties = { ...smallButton, background: "var(--button-primary-bg)", color: "var(--button-primary-fg)", border: "none" };
