import React, { memo, useEffect, useRef, useState } from "react";
import { Brain, ChevronRight, Circle, CircleAlert, CircleCheck, CircleDot, CircleHelp, ShieldAlert, Wrench } from "lucide-react";

import { Markdown } from "../Markdown/Markdown.js";
import type { AssistantItem, AssistantToolCall } from "./assistantApi.js";

/** The conversation: the person's prompts (bubbles), the Assistant's text,
 *  and everything else as rows in the dock panels' style — thinking and tool
 *  calls open in place to their text (a call's status and Undo on its line),
 *  a permission request is a decision row with Allow / Decline, a block or an
 *  error a one-line note with its icon. Follows the end while the person is
 *  at the bottom. */
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
        <div style={{ maxWidth: "85%", marginLeft: "auto", background: "var(--accent-soft-bg)", borderRadius: 8, padding: "6px 10px" }}>
          <div style={{ whiteSpace: "pre-wrap" }}>{item.text}</div>
        </div>
      );
    case "agent":
      return <Markdown value={item.text} />;
    case "thought":
      return <Row icon={<Brain size={13} aria-hidden />} title="Thinking" muted body={<div style={{ whiteSpace: "pre-wrap" }}>{item.text}</div>} />;
    case "tool":
      return <ToolRow call={item.call} undone={undone} onUndo={onUndo} />;
    case "plan":
      return (
        <div style={{ display: "flex", flexDirection: "column", gap: 2, paddingLeft: 2 }}>
          <div style={{ color: "var(--text-muted)", fontWeight: 600 }}>Plan</div>
          {item.entries.map((e, i) => (
            <div key={i} style={{ display: "flex", gap: 6, color: e.status === "completed" ? "var(--text-muted)" : "var(--text-primary)" }}>
              {e.status === "completed" ? (
                <CircleCheck size={13} aria-label="Done" style={{ flexShrink: 0, marginTop: 1, color: "var(--status-done)" }} />
              ) : e.status === "in_progress" ? (
                <CircleDot size={13} aria-label="Working on it" style={{ flexShrink: 0, marginTop: 1, color: "var(--status-running)" }} />
              ) : (
                <Circle size={13} aria-label="To do" style={{ flexShrink: 0, marginTop: 1, color: "var(--text-muted)" }} />
              )}
              <span style={{ textDecoration: e.status === "completed" ? "line-through" : "none" }}>{e.content}</span>
            </div>
          ))}
        </div>
      );
    case "permission":
      return (
        <div data-testid={`assistant-permission-${item.requestId}`} style={{ ...rowFrame, display: "flex", alignItems: "center", gap: 8, padding: "6px 8px" }}>
          <CircleHelp size={13} aria-label="Asks you" style={{ flexShrink: 0, color: "var(--accent)" }} />
          <span style={{ flex: 1, minWidth: 0 }}>
            Asks to run <strong>{item.title}</strong>
          </span>
          {item.answer ? (
            <span style={{ color: "var(--text-muted)" }}>{answerLine(item)}</span>
          ) : (
            <>
              <button type="button" className="ox-button ox-primary" data-testid={`assistant-allow-${item.requestId}`} onClick={() => onAnswer(item.requestId, true)}>
                Allow
              </button>
              <button type="button" className="ox-button" data-testid={`assistant-decline-${item.requestId}`} onClick={() => onAnswer(item.requestId, false)}>
                Decline
              </button>
            </>
          )}
        </div>
      );
    case "policy_denied":
      return <Note tone="var(--status-waiting)" icon={<ShieldAlert size={13} aria-label="Blocked" />} text={<>oxplow blocked {item.label}: {item.reason}</>} />;
    case "bypass":
      return (
        <Note
          tone="var(--severity-high)"
          icon={<ShieldAlert size={13} aria-label="Ran without asking" />}
          text={
            <>
              <strong>{item.label} ran without asking.</strong> oxplow would have blocked it: {item.reason}
            </>
          }
        />
      );
    case "error":
      return <Note testId="assistant-error-item" tone="var(--severity-critical)" icon={<CircleAlert size={13} aria-label="Failed" />} text={item.message} />;
  }
});

function answerLine(item: Extract<AssistantItem, { type: "permission" }>): string {
  const a = item.answer;
  if (!a || a.type === "cancelled") return "Cancelled";
  const kind = item.options.find((o) => o.id === a.optionId)?.kind;
  return kind && kind.startsWith("allow") ? "Allowed" : "Declined";
}

/** A one-line note: an icon in its tone and what it says. */
function Note({ tone, icon, text, testId }: { tone: string; icon: React.ReactNode; text: React.ReactNode; testId?: string }) {
  return (
    <div data-testid={testId} style={{ display: "flex", alignItems: "flex-start", gap: 8, padding: "2px 2px" }}>
      <span style={{ display: "inline-flex", flexShrink: 0, marginTop: 1, color: tone }}>{icon}</span>
      <span style={{ flex: 1, minWidth: 0, color: "var(--text-primary)" }}>{text}</span>
    </div>
  );
}

/** A row that opens in place, as a Context or Alerts row does: a chevron,
 *  an icon, a title, what is at its right; its body under it once open. */
function Row({
  icon,
  title,
  right,
  body,
  muted = false,
  testId,
}: {
  icon: React.ReactNode;
  title: React.ReactNode;
  right?: React.ReactNode;
  body?: React.ReactNode;
  muted?: boolean;
  testId?: string;
}) {
  const [open, setOpen] = useState(false);
  const [hover, setHover] = useState(false);
  const toggles = body != null;
  return (
    <div data-testid={testId} style={rowFrame}>
      <div
        role={toggles ? "button" : undefined}
        tabIndex={toggles ? 0 : undefined}
        aria-expanded={toggles ? open : undefined}
        onClick={() => toggles && setOpen((v) => !v)}
        onKeyDown={(e) => {
          if (toggles && e.target === e.currentTarget && (e.key === "Enter" || e.key === " ")) {
            e.preventDefault();
            setOpen((v) => !v);
          }
        }}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => setHover(false)}
        style={{ display: "flex", alignItems: "center", gap: 8, padding: "4px 8px", cursor: toggles ? "pointer" : "default", background: hover && toggles ? "rgba(255, 255, 255, 0.04)" : "transparent", minWidth: 0 }}
      >
        <ChevronRight
          size={12}
          aria-hidden
          style={{ flexShrink: 0, color: "var(--text-muted)", visibility: toggles ? "visible" : "hidden", transform: open ? "rotate(90deg)" : "none", transition: "transform 150ms ease" }}
        />
        <span style={{ display: "inline-flex", flexShrink: 0, color: "var(--text-muted)" }}>{icon}</span>
        <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: muted ? "var(--text-secondary)" : "var(--text-primary)" }}>{title}</span>
        {right}
      </div>
      {toggles ? (
        <div style={{ display: "grid", gridTemplateRows: open ? "1fr" : "0fr", transition: "grid-template-rows 180ms ease" }}>
          <div inert={!open} aria-hidden={!open} style={{ overflow: "hidden", minHeight: 0 }}>
            <div style={{ padding: "2px 8px 8px 46px", color: "var(--text-secondary)" }}>{body}</div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function ToolRow({ call, undone, onUndo }: { call: AssistantToolCall; undone: ReadonlySet<number>; onUndo(auditId: number): void }) {
  const output = call.text.join("\n").trim();
  const canUndo = call.auditId != null && call.status === "completed";
  return (
    <Row
      testId="assistant-tool"
      icon={<Wrench size={13} aria-label={call.kind} />}
      title={
        <>
          <span style={{ color: "var(--text-muted)" }}>{call.kind} </span>
          {call.title || call.name || "Tool call"}
        </>
      }
      right={
        <>
          <span data-testid="assistant-tool-status" style={{ flexShrink: 0, color: toolStatusColor(call.status) }}>
            {call.status.replace("_", " ")}
          </span>
          {canUndo ? (
            undone.has(call.auditId!) ? (
              <span style={{ flexShrink: 0, color: "var(--text-muted)" }}>undone</span>
            ) : (
              <button
                type="button"
                data-testid="assistant-undo"
                onClick={(e) => {
                  e.stopPropagation();
                  onUndo(call.auditId!);
                }}
                style={linkButton}
              >
                Undo
              </button>
            )
          ) : null}
        </>
      }
      body={output ? <pre style={preStyle}>{output}</pre> : undefined}
    />
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

const rowFrame: React.CSSProperties = { border: "1px solid var(--border-subtle)", borderRadius: 6, background: "rgba(255, 255, 255, 0.02)", overflow: "hidden" };
const preStyle: React.CSSProperties = {
  whiteSpace: "pre-wrap",
  wordBreak: "break-word",
  margin: 0,
  maxHeight: 240,
  overflow: "auto",
  fontSize: 11,
  fontFamily: "var(--font-mono, monospace)",
  borderLeft: "2px solid var(--border-subtle)",
  paddingLeft: 8,
};
const linkButton: React.CSSProperties = { background: "none", border: "none", padding: 0, font: "inherit", color: "var(--accent)", cursor: "pointer", flexShrink: 0 };
