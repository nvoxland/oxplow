import { useEffect, useRef } from "react";

import {
  addCommentMessage,
  deleteComment,
  setCommentIntent,
  setCommentStatus,
} from "../../api.js";
import type { CommentThread } from "../../tauri-bridge/generated/bindings.js";
import { InlineConfirm } from "../InlineConfirm.js";
import { CommentComposer } from "./CommentComposer.js";

const CARD_WIDTH = 420;

const stepButtonStyle: React.CSSProperties = {
  border: "1px solid var(--border-subtle)",
  background: "transparent",
  color: "var(--text-secondary)",
  borderRadius: 4,
  padding: "1px 8px",
  fontSize: "var(--text-xs)",
  cursor: "pointer",
};

/// Whether the stored anchor was a fuzzy (approximate) re-attachment.
export function anchorIsApprox(anchorJson: string): boolean {
  try {
    return (JSON.parse(anchorJson) as { approx?: boolean }).approx === true;
  } catch {
    return false;
  }
}

/// Relative timestamp ("3m ago"). `Timestamp` serializes to an RFC3339
/// string, so `new Date(...)` parses it directly.
function relTime(ts: unknown): string {
  const then = new Date(String(ts)).getTime();
  if (Number.isNaN(then)) return "";
  const secs = Math.max(0, (Date.now() - then) / 1000);
  if (secs < 60) return "just now";
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

/// Floating card showing one comment thread: the anchored quote, the
/// message back-and-forth, the intent toggle, resolve/reopen, delete,
/// and a reply composer. Mutations go straight through the api; the
/// live `CommentsChanged` subscription on the owning surface refetches
/// and re-feeds an updated `thread` prop, so this stays presentational
/// apart from firing the calls.
export function CommentPopover({
  thread,
  author = "user",
  anchorRect,
  onClose,
  onStep,
  onRelink,
}: {
  thread: CommentThread;
  author?: string;
  anchorRect: DOMRect | null;
  onClose: () => void;
  /// Step to the prev/next comment on the page. When provided, ◀ ▶
  /// buttons render in the header; the host scrolls to + reopens the
  /// adjacent comment. Omitted when there's nowhere to step.
  onStep?: (dir: -1 | 1) => void;
  /// Re-attach an orphaned comment to the editor's current selection.
  /// When provided (orphaned + host has a live editor), a "Relink to
  /// selection" button renders.
  onRelink?: () => void;
}) {
  const cardRef = useRef<HTMLDivElement | null>(null);
  const { comment, messages } = thread;
  const resolved = comment.status === "resolved";
  const approx = !comment.orphaned && anchorIsApprox(comment.selectors_json);

  // Close on outside click.
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (cardRef.current && !cardRef.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [onClose]);

  // Position below-left of the anchor, clamped to the viewport.
  const left = anchorRect
    ? Math.min(Math.max(8, anchorRect.left), window.innerWidth - CARD_WIDTH - 8)
    : window.innerWidth - CARD_WIDTH - 16;
  const top = anchorRect ? Math.min(anchorRect.bottom + 6, window.innerHeight - 80) : 64;

  return (
    <div
      ref={cardRef}
      data-testid={`comment-popover-${comment.id}`}
      // Stop pointer events from bubbling to a host editor wrapper
      // (RichTextField's `.oxplow-rt-field` onClick refocuses the editor),
      // which would steal focus from the reply box.
      onMouseDown={(e) => e.stopPropagation()}
      onMouseUp={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      style={{
        position: "fixed",
        left,
        top,
        width: CARD_WIDTH,
        maxHeight: "60vh",
        overflowY: "auto",
        zIndex: 1000,
        background: "var(--surface-elevated)",
        border: "1px solid var(--border-strong)",
        borderRadius: 8,
        boxShadow: "0 8px 24px rgba(0,0,0,0.4)",
        padding: 12,
        display: "flex",
        flexDirection: "column",
        gap: 10,
        fontFamily: "var(--font-ui)",
      }}
    >
      {/* Prev/next stepper across the page's comments */}
      {onStep ? (
        <div style={{ display: "flex", alignItems: "center", gap: 6, justifyContent: "flex-end" }}>
          <button
            type="button"
            data-testid={`comment-step-prev-${comment.id}`}
            title="Previous comment on this page"
            onClick={() => onStep(-1)}
            style={stepButtonStyle}
          >
            ◀ Prev
          </button>
          <button
            type="button"
            data-testid={`comment-step-next-${comment.id}`}
            title="Next comment on this page"
            onClick={() => onStep(1)}
            style={stepButtonStyle}
          >
            Next ▶
          </button>
        </div>
      ) : null}

      {/* Anchored quote */}
      <div
        style={{
          fontSize: "var(--text-xs)",
          color: "var(--text-secondary)",
          borderLeft: "2px solid var(--comment-highlight)",
          paddingLeft: 8,
          fontStyle: "italic",
          maxHeight: 60,
          overflow: "hidden",
        }}
      >
        {comment.orphaned && (
          <span style={{ color: "var(--freshness-stale)", fontStyle: "normal" }}>
            (orphaned){" "}
          </span>
        )}
        {approx && (
          <span
            title="Re-attached approximately — the quoted text drifted, so this anchor may not be exact."
            style={{ color: "var(--freshness-stale)", fontStyle: "normal" }}
          >
            (approx){" "}
          </span>
        )}
        “{comment.quote}”
        {comment.orphaned && (
          <div style={{ marginTop: 6, fontStyle: "normal", display: "flex", flexDirection: "column", gap: 4 }}>
            <span style={{ color: "var(--text-muted)" }}>
              Anchor lost. Select the intended text in the editor, then relink.
            </span>
            {onRelink ? (
              <button
                type="button"
                data-testid={`comment-relink-${comment.id}`}
                onClick={() => onRelink()}
                title="Re-attach this comment to the text currently selected in the editor"
                style={{
                  alignSelf: "flex-start",
                  border: "1px solid var(--border-subtle)",
                  background: "transparent",
                  color: "var(--accent)",
                  borderRadius: 4,
                  padding: "2px 8px",
                  fontSize: "var(--text-xs)",
                  cursor: "pointer",
                }}
              >
                Relink to selection
              </button>
            ) : null}
          </div>
        )}
      </div>

      {/* Message thread */}
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        {messages.map((m) => (
          <div key={m.id} data-testid={`comment-message-${m.id}`}>
            <div style={{ display: "flex", gap: 6, alignItems: "baseline" }}>
              <span
                style={{
                  fontSize: "var(--text-xs)",
                  fontWeight: 600,
                  color: m.author === "agent" ? "var(--accent)" : "var(--text-primary)",
                }}
              >
                {m.author}
              </span>
              <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>
                {relTime(m.created_at)}
              </span>
            </div>
            <div
              style={{
                fontSize: "var(--text-sm)",
                color: "var(--text-primary)",
                whiteSpace: "pre-wrap",
              }}
            >
              {m.body}
            </div>
          </div>
        ))}
      </div>

      {/* Reply composer (hidden once resolved) */}
      {!resolved && (
        <CommentComposer
          submitLabel="Reply"
          placeholder="Reply…"
          testIdPrefix={`comment-reply-${comment.id}`}
          onSubmit={async (body) => {
            await addCommentMessage(comment.id, author, body);
          }}
          onCancel={onClose}
        />
      )}

      {/* Footer actions */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          borderTop: "1px solid var(--border-subtle)",
          paddingTop: 8,
        }}
      >
        <select
          value={comment.intent}
          data-testid={`comment-intent-${comment.id}`}
          onChange={(e) =>
            void setCommentIntent(comment.id, e.target.value as CommentThread["comment"]["intent"])
          }
          style={{
            padding: "3px 6px",
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
        <div style={{ flex: 1 }} />
        <button
          type="button"
          data-testid={`comment-resolve-${comment.id}`}
          onClick={() => void setCommentStatus(comment.id, resolved ? "open" : "resolved")}
          style={{
            padding: "3px 10px",
            background: "transparent",
            color: resolved ? "var(--status-done)" : "var(--text-secondary)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 6,
            fontSize: "var(--text-xs)",
            cursor: "pointer",
          }}
        >
          {resolved ? "Reopen" : "Resolve"}
        </button>
        <InlineConfirm
          triggerLabel="Delete"
          testIdPrefix={`comment-delete-${comment.id}`}
          onConfirm={() => {
            void deleteComment(comment.id).then(onClose);
          }}
          triggerStyle={{
            padding: "3px 10px",
            background: "transparent",
            color: "var(--text-secondary)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 6,
            fontSize: "var(--text-xs)",
            cursor: "pointer",
          }}
        />
      </div>
    </div>
  );
}
