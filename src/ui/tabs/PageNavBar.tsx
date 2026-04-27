import type { ReactNode } from "react";
import { useState } from "react";
import type { BookmarkScope } from "./bookmarks.js";

export interface PageNavBarProps {
  canBack: boolean;
  canForward: boolean;
  onBack(): void;
  onForward(): void;
  /** Optional bookmark affordance — when omitted, no star renders.
   *  The button always opens a popover that lets the user toggle this
   *  page's bookmark in each scope (thread / stream / global). */
  bookmark?: {
    /** Scopes this page is currently bookmarked at. The star is filled
     *  when this is non-empty. */
    scopes: BookmarkScope[];
    onToggleScope(scope: BookmarkScope): void;
  };
  /** Optional backlinks dropdown content — when omitted, no dropdown renders. */
  backlinks?: {
    count: number;
    body: ReactNode;
  };
  /** Optional kebab actions slot at the right edge. */
  actions?: ReactNode;
}

/**
 * Browser-style navigation bar rendered inside `Page` chrome. Carries
 * back/forward, an optional bookmark toggle, and an optional backlinks
 * dropdown. Kept dumb and props-driven — the host wires it to the
 * real navigation/bookmark/backlinks state.
 */
export function PageNavBar({
  canBack,
  canForward,
  onBack,
  onForward,
  bookmark,
  backlinks,
  actions,
}: PageNavBarProps) {
  const [backlinksOpen, setBacklinksOpen] = useState(false);
  const [scopeOpen, setScopeOpen] = useState(false);

  return (
    <div
      data-testid="page-nav-bar"
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "6px 12px",
        borderBottom: "1px solid var(--border-subtle)",
        background: "var(--surface-app)",
        flexShrink: 0,
        minHeight: 36,
        position: "relative",
      }}
    >
      <button
        type="button"
        data-testid="page-nav-back"
        title="Back"
        disabled={!canBack}
        onClick={onBack}
        style={navButtonStyle(canBack)}
      >
        ←
      </button>
      <button
        type="button"
        data-testid="page-nav-forward"
        title="Forward"
        disabled={!canForward}
        onClick={onForward}
        style={navButtonStyle(canForward)}
      >
        →
      </button>

      {bookmark ? (
        <div style={{ position: "relative", display: "inline-flex" }}>
          <button
            type="button"
            data-testid="page-nav-bookmark"
            title="Bookmark"
            onClick={() => setScopeOpen((v) => !v)}
            aria-expanded={scopeOpen}
            style={{
              ...navButtonStyle(true),
              color: bookmark.scopes.length > 0 ? "var(--accent-fg)" : "var(--text-secondary)",
            }}
          >
            {bookmark.scopes.length > 0 ? "★" : "☆"}
          </button>
          {scopeOpen ? (
            <div
              data-testid="page-nav-bookmark-popover"
              style={{
                position: "absolute",
                top: "calc(100% + 4px)",
                left: 0,
                minWidth: 180,
                background: "var(--surface-card)",
                border: "1px solid var(--border-subtle)",
                borderRadius: 6,
                boxShadow: "0 4px 12px rgba(0,0,0,0.18)",
                padding: 6,
                zIndex: 10,
                fontSize: 12,
              }}
            >
              {(["thread", "stream", "global"] as BookmarkScope[]).map((scope) => {
                const active = bookmark.scopes.includes(scope);
                return (
                  <button
                    key={scope}
                    type="button"
                    data-testid={`page-nav-bookmark-scope-${scope}`}
                    onClick={() => bookmark.onToggleScope(scope)}
                    style={{
                      display: "block",
                      width: "100%",
                      textAlign: "left",
                      padding: "4px 6px",
                      background: "transparent",
                      border: "none",
                      color: "var(--text-primary)",
                      cursor: "pointer",
                      borderRadius: 4,
                    }}
                  >
                    <span style={{ display: "inline-block", width: 14 }}>
                      {active ? "★" : " "}
                    </span>
                    {scope === "thread" ? "This thread" : scope === "stream" ? "This stream" : "Global"}
                  </button>
                );
              })}
            </div>
          ) : null}
        </div>
      ) : null}

      {backlinks ? (
        <div style={{ position: "relative" }}>
          <button
            type="button"
            data-testid="page-nav-backlinks-toggle"
            onClick={() => setBacklinksOpen((v) => !v)}
            disabled={backlinks.count === 0}
            aria-expanded={backlinksOpen}
            style={{
              ...navButtonStyle(backlinks.count > 0),
              padding: "4px 10px",
              fontSize: 12,
            }}
          >
            Backlinks ({backlinks.count}) {backlinksOpen ? "▾" : "▸"}
          </button>
          {backlinksOpen && backlinks.count > 0 ? (
            <div
              data-testid="page-nav-backlinks-popover"
              style={{
                position: "absolute",
                top: "calc(100% + 4px)",
                left: 0,
                minWidth: 280,
                maxWidth: 480,
                maxHeight: 360,
                overflow: "auto",
                background: "var(--surface-card)",
                border: "1px solid var(--border-subtle)",
                borderRadius: 6,
                boxShadow: "0 4px 12px rgba(0,0,0,0.18)",
                padding: 8,
                zIndex: 10,
              }}
            >
              {backlinks.body}
            </div>
          ) : null}
        </div>
      ) : null}

      <div style={{ flex: 1 }} />

      {actions ? (
        <div style={{ display: "flex", gap: 6, flexShrink: 0 }}>{actions}</div>
      ) : null}
    </div>
  );
}

function navButtonStyle(enabled: boolean): React.CSSProperties {
  return {
    border: "1px solid var(--border-subtle)",
    background: "var(--surface-card)",
    color: enabled ? "var(--text-primary)" : "var(--text-secondary)",
    padding: "4px 8px",
    borderRadius: 4,
    cursor: enabled ? "pointer" : "default",
    opacity: enabled ? 1 : 0.4,
    fontSize: 13,
    minWidth: 28,
  };
}
