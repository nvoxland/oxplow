import { useEffect, useRef, useState } from "react";

import { requestCommentReveal } from "../../comment-reveal-bus.js";
import { partitionPageComments } from "./pageCommentNav.js";
import { useCommentsForTarget } from "./useCommentsForTarget.js";

function quoteSnippet(quote: string): string {
  const q = quote.replace(/\s+/g, " ").trim();
  return q.length > 48 ? `${q.slice(0, 48)}…` : q;
}

/// Comment navigator for the page nav bar. Shows how many comments are
/// on the page, steps through the anchored ones (◀ ▶, scrolling/opening
/// each via the reveal bus), and lists everything — including orphaned
/// comments that have no inline highlight — in a dropdown. Renders
/// nothing when the page has no comments.
export function CommentNavigator({
  target,
}: {
  target: string;
}) {
  const { threads } = useCommentsForTarget(target);
  const [open, setOpen] = useState(false);
  const [idx, setIdx] = useState(0);
  // Default to unresolved only (mirrors the Comments Dashboard); the
  // dropdown carries a toggle to include resolved threads.
  const [showResolved, setShowResolved] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  const hasResolved = threads.some((t) => t.comment.status === "resolved");
  const shown = showResolved ? threads : threads.filter((t) => t.comment.status !== "resolved");
  const { jumpable, orphaned, total } = partitionPageComments(shown);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  if (threads.length === 0) return null;

  const jumpTo = (i: number) => {
    if (jumpable.length === 0) return;
    const wrapped = ((i % jumpable.length) + jumpable.length) % jumpable.length;
    setIdx(wrapped);
    requestCommentReveal(jumpable[wrapped].comment.id);
  };

  return (
    <div ref={rootRef} style={{ position: "relative", display: "inline-flex", alignItems: "center", gap: 2 }}>
      <button
        type="button"
        data-testid="page-nav-comments-prev"
        title="Previous comment"
        disabled={jumpable.length === 0}
        onClick={() => jumpTo(idx - 1)}
        style={stepBtn(jumpable.length > 0)}
      >
        ◀
      </button>
      <button
        type="button"
        data-testid="page-nav-comments-toggle"
        title="Comments on this page"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        style={{
          border: "1px solid var(--border-subtle)",
          background: "var(--surface-card)",
          color: "var(--text-primary)",
          padding: "4px 8px",
          borderRadius: 4,
          cursor: "pointer",
          fontSize: "var(--text-xs)",
        }}
      >
        Comments ({total}) {open ? "▾" : "▸"}
      </button>
      <button
        type="button"
        data-testid="page-nav-comments-next"
        title="Next comment"
        disabled={jumpable.length === 0}
        onClick={() => jumpTo(idx + 1)}
        style={stepBtn(jumpable.length > 0)}
      >
        ▶
      </button>

      {open ? (
        <div
          data-testid="page-nav-comments-popover"
          style={{
            position: "absolute",
            top: "calc(100% + 4px)",
            right: 0,
            minWidth: 260,
            maxWidth: 420,
            maxHeight: 360,
            overflow: "auto",
            background: "var(--surface-card)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 6,
            boxShadow: "0 4px 12px rgba(0,0,0,0.18)",
            padding: 4,
            zIndex: 10,
            fontSize: "var(--text-xs)",
          }}
        >
          {hasResolved ? (
            <div
              style={{
                display: "flex",
                justifyContent: "flex-end",
                padding: "2px 4px 4px",
                borderBottom: "1px solid var(--border-subtle)",
                marginBottom: 4,
              }}
            >
              <button
                type="button"
                data-testid="page-nav-comments-show-resolved"
                onClick={() => setShowResolved((v) => !v)}
                style={{
                  border: "1px solid var(--border-subtle)",
                  background: "transparent",
                  color: "var(--text-secondary)",
                  borderRadius: 4,
                  padding: "1px 8px",
                  fontSize: 10,
                  cursor: "pointer",
                }}
              >
                {showResolved ? "Unresolved only" : "Show resolved"}
              </button>
            </div>
          ) : null}
          {total === 0 ? (
            <div style={{ padding: "6px 8px", color: "var(--text-muted)" }}>
              No unresolved comments.
            </div>
          ) : null}
          {jumpable.map((t) => (
            <button
              key={t.comment.id}
              type="button"
              data-testid={`page-nav-comments-item-${t.comment.id}`}
              onClick={() => {
                setOpen(false);
                requestCommentReveal(t.comment.id);
              }}
              style={itemStyle}
              title={t.comment.quote}
            >
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                “{quoteSnippet(t.comment.quote)}”
              </span>
              {t.comment.status === "resolved" ? (
                <span style={{ color: "var(--status-done)", flexShrink: 0 }}>resolved</span>
              ) : t.comment.intent === "followup" ? (
                <span style={{ color: "var(--accent)", flexShrink: 0 }}>follow-up</span>
              ) : null}
            </button>
          ))}

          {orphaned.length > 0 ? (
            <>
              <div
                style={{
                  padding: "6px 8px 2px",
                  fontSize: 10,
                  textTransform: "uppercase",
                  letterSpacing: 0.4,
                  color: "var(--freshness-stale)",
                }}
                title="These comments' anchored text changed too much to locate. Select the intended text and right-click → “Relink orphaned” to re-attach."
              >
                Orphaned ({orphaned.length})
              </div>
              {orphaned.map((t) => (
                <button
                  key={t.comment.id}
                  type="button"
                  data-testid={`page-nav-comments-orphaned-${t.comment.id}`}
                  onClick={() => {
                    setOpen(false);
                    requestCommentReveal(t.comment.id);
                  }}
                  style={{ ...itemStyle, color: "var(--text-muted)" }}
                  title={`Orphaned — open to read it and relink. (${t.comment.quote})`}
                >
                  <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    “{quoteSnippet(t.comment.quote)}”
                  </span>
                </button>
              ))}
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

const itemStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  width: "100%",
  textAlign: "left",
  padding: "5px 8px",
  background: "transparent",
  border: "none",
  color: "var(--text-primary)",
  cursor: "pointer",
  borderRadius: 4,
};

function stepBtn(enabled: boolean): React.CSSProperties {
  return {
    border: "1px solid var(--border-subtle)",
    background: "var(--surface-card)",
    color: enabled ? "var(--text-primary)" : "var(--text-secondary)",
    padding: "4px 6px",
    borderRadius: 4,
    cursor: enabled ? "pointer" : "default",
    opacity: enabled ? 1 : 0.4,
    fontSize: 10,
    minWidth: 22,
  };
}
