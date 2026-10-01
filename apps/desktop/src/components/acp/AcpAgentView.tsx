import React, { useCallback, useEffect, useRef, useState } from "react";

import {
  acpCancel,
  acpDismissDirective,
  acpOpenSession,
  acpRespondPermission,
  acpTranscript,
  onRemoteReconnect,
  subscribeAcpEvents,
  type AcpToolCall,
  type Thread,
  type TranscriptItem,
} from "../../api.js";
import { WORKING } from "../../revision.js";
import type { TabRef } from "../../tabs/tabState.js";
import type { DiffSpec } from "../Diff/DiffPane.js";
import { answerOfTool, type AnswerRow } from "../../threadAnswers.js";
import { ThreadAnswer } from "../Answers/ThreadAnswer.js";
import { useThreadAnswers } from "../Answers/useThreadAnswers.js";
import { MarkdownView } from "../Wiki/MarkdownView.js";
import { AcpPromptBox } from "./AcpPromptBox.js";
import {
  applyEvent,
  contextPercent,
  initialState,
  isBusy,
  mergeSnapshot,
  type AcpViewState,
} from "./acpTranscript.js";
import { EmptyState } from "../Prompts/EmptyState.js";


interface Props {
  thread: Thread;
  /** The stream's worktree, to show paths relative to it. */
  worktreePath?: string;
  visible: boolean;
  onOpenDiff?(spec: DiffSpec): void;
  /** Open a file by absolute path. */
  onOpenFile?(absPath: string): void;
  onOpenSettings?(): void;
  /** Open a page (an answer's links). */
  onOpenPage?(ref: TabRef): void;
}

/**
 * An ACP thread: the agent's conversation as structured items (messages,
 * tool calls with diffs, the plan, permission cards, policy notices) and
 * a prompt box. It replaces the terminal for `agent: acp` threads.
 *
 * oxplow never sends the agent anything on its own. The turn-end
 * directive shows as a banner; "Put in input" only fills the draft, and
 * only the person's Enter sends (see AcpPromptBox).
 */
export function AcpAgentView({ thread, worktreePath, visible, onOpenDiff, onOpenFile, onOpenSettings, onOpenPage }: Props) {
  const threadId = thread.id;
  const [state, setState] = useState<AcpViewState>(initialState);
  const [draft, setDraft] = useState("");
  const [openError, setOpenError] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const headSeq = useRef(0);
  headSeq.current = state.headSeq;

  const open = useCallback(async () => {
    setOpening(true);
    setOpenError(null);
    try {
      const snap = await acpOpenSession(threadId);
      setState((s) => mergeSnapshot({ ...s, closedReason: null }, snap));
    } catch (err) {
      setOpenError(err instanceof Error ? err.message : String(err));
    } finally {
      setOpening(false);
    }
  }, [threadId]);

  const refetch = useCallback(async () => {
    try {
      const snap = await acpTranscript(threadId, headSeq.current);
      if (snap) setState((s) => mergeSnapshot(s, snap));
    } catch {
      // The next event or reconnect retries.
    }
  }, [threadId]);

  // Events first, then the snapshot, so nothing between the two is lost
  // (duplicates merge by id and seq).
  useEffect(() => {
    setState(initialState());
    const unsubscribe = subscribeAcpEvents((e) => {
      if (e.threadId !== threadId) return;
      setState((s) => applyEvent(s, e));
    });
    let cancelled = false;
    void (async () => {
      try {
        const snap = await acpTranscript(threadId, 0);
        if (cancelled) return;
        if (snap) setState((s) => mergeSnapshot(s, snap));
        else void open();
      } catch (err) {
        if (!cancelled) setOpenError(err instanceof Error ? err.message : String(err));
      }
    })();
    const offReconnect = onRemoteReconnect(() => void refetch());
    return () => {
      cancelled = true;
      unsubscribe();
      offReconnect();
    };
  }, [threadId, open, refetch]);

  useEffect(() => {
    if (state.stale) void refetch();
  }, [state.stale, refetch]);

  const busy = isBusy(state.status);
  const live = state.status !== "stopped" && state.status !== "starting";
  const pct = contextPercent(state.usage);

  const report = (err: unknown) => setActionError(err instanceof Error ? err.message : String(err));
  const cancel = () => void acpCancel(threadId).catch(report);

  const relPath = (p: string) => {
    const wt = worktreePath?.replace(/\/$/, "");
    return wt && p.startsWith(wt + "/") ? p.slice(wt.length + 1) : p;
  };

  return (
    <div
      data-testid="acp-view"
      style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0, background: "var(--surface-app)" }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 10,
          padding: "6px 10px",
          borderBottom: "1px solid var(--border-subtle)",
          background: "var(--surface-card)",
          fontSize: 12,
        }}
      >
        <span style={{ color: "var(--text-primary)", fontWeight: 600 }}>ACP · {thread.acp_agent ?? state.agent}</span>
        <span data-testid="acp-status" style={{ color: statusColor(state.status) }}>
          {statusLabel(state.status)}
        </span>
        <span style={{ flex: 1 }} />
        {pct !== null && state.usage && (
          <span
            data-testid="acp-context-meter"
            title={`${state.usage.used.toLocaleString()} of ${state.usage.size.toLocaleString()} tokens${
              state.usage.costAmount !== null ? ` · ${state.usage.costAmount.toFixed(2)} ${state.usage.costCurrency ?? ""}` : ""
            }`}
            style={{ color: "var(--text-secondary)" }}
          >
            Context {pct}%
          </span>
        )}
        {state.status === "stopped" && (
          <button type="button" data-testid="acp-restart" onClick={() => void open()} style={smallButton} disabled={opening}>
            {opening ? "Starting…" : "Restart"}
          </button>
        )}
      </div>

      {openError && (
        <div data-testid="acp-open-error" style={{ ...notice("var(--severity-high)"), margin: 10 }}>
          <div style={{ whiteSpace: "pre-wrap" }}>{openError}</div>
          <div style={{ marginTop: 6, display: "flex", gap: 8 }}>
            <button type="button" data-testid="acp-retry" onClick={() => void open()} style={smallButton} disabled={opening}>
              {opening ? "Starting…" : "Retry"}
            </button>
            {/approve|Programs/.test(openError) && onOpenSettings && (
              <button type="button" data-testid="acp-open-settings" onClick={onOpenSettings} style={smallButton}>
                Open settings
              </button>
            )}
          </div>
        </div>
      )}

      <Transcript
        items={state.items}
        threadId={threadId}
        relPath={relPath}
        onOpenDiff={onOpenDiff}
        onOpenFile={onOpenFile}
        onOpenPage={onOpenPage}
        onError={report}
        starting={opening || state.status === "starting"}
      />

      {state.status === "stopped" && state.closedReason && (
        <div style={{ ...notice("var(--text-muted)"), margin: "0 10px 6px" }}>
          The session ended: {state.closedReason}
          {state.stderrTail.length > 0 && (
            <details>
              <summary>Agent output</summary>
              <pre style={preStyle}>{state.stderrTail.join("\n")}</pre>
            </details>
          )}
        </div>
      )}

      {state.directive && (
        <div data-testid="acp-directive" style={{ ...notice("var(--status-waiting)"), margin: "0 10px 6px" }}>
          <div style={{ fontWeight: 600, marginBottom: 4 }}>oxplow reminder (not sent to the agent)</div>
          <div style={{ whiteSpace: "pre-wrap" }}>{state.directive}</div>
          <div style={{ marginTop: 6, display: "flex", gap: 8 }}>
            <button
              type="button"
              data-testid="acp-directive-put"
              style={smallButton}
              onClick={() => {
                const text = state.directive ?? "";
                setDraft((d) => (d.trim() ? `${d}\n\n${text}` : text));
              }}
            >
              Put in input
            </button>
            <button
              type="button"
              data-testid="acp-directive-dismiss"
              style={smallButton}
              onClick={() => void acpDismissDirective(threadId).catch(report)}
            >
              Dismiss
            </button>
          </div>
        </div>
      )}

      {actionError && (
        <div data-testid="acp-action-error" style={{ ...notice("var(--severity-high)"), margin: "0 10px 6px" }}>
          {actionError}{" "}
          <button type="button" style={smallButton} onClick={() => setActionError(null)}>
            Dismiss
          </button>
        </div>
      )}

      <AcpPromptBox
        threadId={threadId}
        draft={draft}
        setDraft={setDraft}
        busy={busy}
        disabled={!live}
        visible={visible}
        onCancel={cancel}
        onError={(m) => setActionError(m)}
      />
    </div>
  );
}

function Transcript({
  items,
  threadId,
  relPath,
  onOpenDiff,
  onOpenFile,
  onOpenPage,
  onError,
  starting,
}: {
  items: TranscriptItem[];
  threadId: string;
  relPath(p: string): string;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenFile?(absPath: string): void;
  onOpenPage?(ref: TabRef): void;
  onError(err: unknown): void;
  /** The session is starting: said plainly — a loading state isn't an
   *  empty one. */
  starting: boolean;
}) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const pinned = useRef(true);
  const answerList = useThreadAnswers(threadId);
  const answers = new Map(answerList.map((a) => [a.ref, a]));
  const last = items[items.length - 1];
  // Follow the conversation while the person is at the bottom.
  useEffect(() => {
    const el = scrollRef.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [items.length, last?.seq]);

  return (
    <div
      ref={scrollRef}
      data-testid="acp-transcript"
      onScroll={(e) => {
        const el = e.currentTarget;
        pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
      }}
      style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: "10px 12px", display: "flex", flexDirection: "column", gap: 8 }}
    >
      {items.length === 0 && starting ? (
        <div style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>Starting the agent…</div>
      ) : items.length === 0 ? (
        <EmptyState
          compact
          title="No messages yet"
          prompts={["What changed in this project this week?", "What's in progress right now?"]}
        />
      ) : (
        items.map((item) => (
          <div key={item.id} data-testid={`acp-item-${item.id}`} data-kind={item.type}>
            <Item
              item={item}
              threadId={threadId}
              answers={answers}
              onOpenPage={onOpenPage}
              relPath={relPath}
              onOpenDiff={onOpenDiff}
              onOpenFile={onOpenFile}
              onError={onError}
            />
          </div>
        ))
      )}
    </div>
  );
}

function Item({
  item,
  threadId,
  answers,
  onOpenPage,
  relPath,
  onOpenDiff,
  onOpenFile,
  onError,
}: {
  item: TranscriptItem;
  threadId: string;
  /** The thread's answers by ref: a `show_lens` call renders its own. */
  answers: ReadonlyMap<string, AnswerRow>;
  onOpenPage?(ref: TabRef): void;
  relPath(p: string): string;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenFile?(absPath: string): void;
  onError(err: unknown): void;
}) {
  switch (item.type) {
    case "user":
      return (
        <div style={{ alignSelf: "flex-end", maxWidth: "85%", marginLeft: "auto", ...bubble("var(--accent-soft-bg)") }}>
          <div style={{ whiteSpace: "pre-wrap" }}>{item.text}</div>
          {item.context && (
            <details style={{ marginTop: 4, color: "var(--text-secondary)", fontSize: 11 }}>
              <summary>oxplow context</summary>
              <pre style={preStyle}>{item.context}</pre>
            </details>
          )}
        </div>
      );
    case "agent":
      return (
        <div style={{ color: "var(--text-primary)" }}>
          <MarkdownView body={item.text} />
        </div>
      );
    case "thought":
      return (
        <details style={{ color: "var(--text-secondary)", fontSize: 12 }}>
          <summary>Thinking</summary>
          <div style={{ whiteSpace: "pre-wrap" }}>{item.text}</div>
        </details>
      );
    case "tool": {
      const answerRef = answerOfTool(item.call);
      const answer = answerRef === null ? undefined : answers.get(answerRef);
      return (
        <>
          <ToolCard call={item.call} itemId={item.id} relPath={relPath} onOpenDiff={onOpenDiff} onOpenFile={onOpenFile} />
          {answer ? (
            <div style={{ marginTop: 6 }}>
              <ThreadAnswer answer={answer} onOpenPage={onOpenPage} />
            </div>
          ) : null}
        </>
      );
    }
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
        <div data-testid={`acp-permission-${item.requestId}`} style={notice("var(--status-waiting)")}>
          <div style={{ marginBottom: 6 }}>
            The agent asks to run <strong>{item.title}</strong>
          </div>
          {item.answer ? (
            <div style={{ color: "var(--text-secondary)" }}>
              {item.answer.type === "cancelled"
                ? "Cancelled"
                : `Answered: ${item.options.find((o) => o.id === (item.answer as { optionId: string }).optionId)?.name ?? "—"}`}
            </div>
          ) : (
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
              {item.options.map((o) => (
                <button
                  key={o.id}
                  type="button"
                  data-testid={`acp-permission-${item.requestId}-${o.id}`}
                  style={o.kind.startsWith("allow") ? primaryButton : smallButton}
                  onClick={() => void acpRespondPermission(threadId, item.requestId, o.id).catch(onError)}
                >
                  {o.name}
                </button>
              ))}
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
        <div data-testid="acp-bypass" style={notice("var(--severity-high)")}>
          <strong>{item.label} ran without asking.</strong> oxplow would have blocked it: {item.reason}
        </div>
      );
    case "directive":
      return (
        <div style={{ color: "var(--text-muted)", fontSize: 11 }}>
          oxplow showed a turn-end reminder (not sent to the agent).
        </div>
      );
    case "error":
      return <div style={notice("var(--severity-critical)")}>{item.message}</div>;
  }
}

function ToolCard({
  call,
  itemId,
  relPath,
  onOpenDiff,
  onOpenFile,
}: {
  call: AcpToolCall;
  itemId: number;
  relPath(p: string): string;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenFile?(absPath: string): void;
}) {
  const output = call.text.join("\n").trim();
  return (
    <div style={{ ...bubble("var(--surface-card)"), fontSize: 12 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "baseline" }}>
        <span style={{ color: "var(--text-secondary)" }}>{call.kind}</span>
        <span style={{ color: "var(--text-primary)", flex: 1 }}>{relTitle(call.title || call.name || "Tool call", relPath)}</span>
        <span style={{ color: toolStatusColor(call.status) }}>{call.status.replace("_", " ")}</span>
      </div>
      {call.locations.length > 0 && (
        <div style={{ marginTop: 2 }}>
          {call.locations.map((p) => (
            <button key={p} type="button" style={linkButton} onClick={() => onOpenFile?.(p)} title={p}>
              {relPath(p)}
            </button>
          ))}
        </div>
      )}
      {call.diffs.map((d, i) => (
        <div key={i} style={{ marginTop: 4, display: "flex", gap: 8, alignItems: "center" }}>
          <span style={{ color: "var(--diff-add-fg)" }}>{d.oldText === null ? "new file" : "edit"}</span>
          <span style={{ flex: 1 }}>{relPath(d.path)}</span>
          {onOpenDiff && (
            <button
              type="button"
              data-testid={`acp-view-diff-${itemId}-${i}`}
              style={smallButton}
              onClick={() =>
                onOpenDiff({
                  path: relPath(d.path),
                  leftVersion: WORKING,
                  rightVersion: WORKING,
                  baseLabel: "before",
                  leftContent: d.oldText ?? "",
                  rightContent: d.newText,
                  labelOverride: `agent edit (${call.id})`,
                })
              }
            >
              View diff
            </button>
          )}
        </div>
      ))}
      {output && (
        <details style={{ marginTop: 4 }}>
          <summary style={{ color: "var(--text-secondary)" }}>Output</summary>
          <pre style={preStyle}>{output}</pre>
        </details>
      )}
    </div>
  );
}

/** Agents put absolute paths in titles; show worktree paths relative. */
function relTitle(title: string, relPath: (p: string) => string): string {
  return title
    .split(" ")
    .map((w) => (w.startsWith("/") ? relPath(w) : w))
    .join(" ");
}

function statusLabel(s: AcpViewState["status"]): string {
  switch (s) {
    case "starting":
      return "Starting";
    case "idle":
      return "Ready";
    case "running":
      return "Working";
    case "awaiting_permission":
      return "Waiting for you";
    case "stopped":
      return "Stopped";
  }
}

function statusColor(s: AcpViewState["status"]): string {
  switch (s) {
    case "running":
      return "var(--status-running)";
    case "awaiting_permission":
      return "var(--status-waiting)";
    case "stopped":
      return "var(--status-canceled)";
    default:
      return "var(--status-ready)";
  }
}

function toolStatusColor(s: AcpToolCall["status"]): string {
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

function bubble(bg: string): React.CSSProperties {
  return {
    background: bg,
    border: "1px solid var(--border-subtle)",
    borderRadius: 6,
    padding: "6px 10px",
  };
}

function notice(color: string): React.CSSProperties {
  return {
    borderLeft: `3px solid ${color}`,
    background: "var(--surface-card)",
    padding: "6px 10px",
    borderRadius: 4,
    color: "var(--text-primary)",
    fontSize: 12,
  };
}

const preStyle: React.CSSProperties = {
  whiteSpace: "pre-wrap",
  margin: "4px 0 0",
  maxHeight: 240,
  overflow: "auto",
  fontSize: 11,
  color: "var(--text-secondary)",
};

const smallButton: React.CSSProperties = {
  background: "transparent",
  color: "var(--text-primary)",
  border: "1px solid var(--border-strong)",
  borderRadius: 4,
  padding: "2px 10px",
  fontSize: 12,
  cursor: "pointer",
};

const primaryButton: React.CSSProperties = {
  ...smallButton,
  background: "var(--button-primary-bg)",
  color: "var(--button-primary-fg)",
  border: "none",
};

const linkButton: React.CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  marginRight: 8,
  color: "var(--accent)",
  cursor: "pointer",
  fontSize: 12,
};
