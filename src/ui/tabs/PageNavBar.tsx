import type { ReactNode } from "react";
import { useState } from "react";
import type { BookmarkScope } from "./bookmarks.js";

export interface PageNavBarProps {
  canBack: boolean;
  canForward: boolean;
  onBack(): void;
  onForward(): void;
  /** Optional bookmark affordance — when omitted, no star renders. */
  bookmark?: {
    isBookmarked: boolean;
    /** Click on the star: toggles in `defaultScope`. */
    onToggle(): void;
    /** Optional scope chooser (chevron). When supplied, the chevron
     *  opens a popover; clicking a scope toggles that scope. */
    scopes?: BookmarkScope[];
    defaultScope?: BookmarkScope;
    onToggleScope?(scope: BookmarkScope): void;
    onSetDefaultScope?(scope: BookmarkScope): void;
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
            title={bookmark.isBookmarked ? "Remove bookmark" : `Bookmark (${bookmark.defaultScope ?? "thread"})`}
            onClick={bookmark.onToggle}
            style={{
              ...navButtonStyle(true),
              color: bookmark.isBookmarked ? "var(--accent-fg)" : "var(--text-secondary)",
              borderTopRightRadius: bookmark.scopes ? 0 : 4,
              borderBottomRightRadius: bookmark.scopes ? 0 : 4,
            }}
          >
            {bookmark.isBookmarked ? "★" : "☆"}
          </button>
          {bookmark.scopes ? (
            <button
              type="button"
              data-testid="page-nav-bookmark-scope"
              title="Choose bookmark scope"
              onClick={() => setScopeOpen((v) => !v)}
              aria-expanded={scopeOpen}
              style={{
                ...navButtonStyle(true),
                marginLeft: -1,
                borderTopLeftRadius: 0,
                borderBottomLeftRadius: 0,
                padding: "4px 6px",
                fontSize: 10,
              }}
            >
              ▾
            </button>
          ) : null}
          {scopeOpen && bookmark.scopes ? (
            <div
              data-testid="page-nav-bookmark-popover"
              style={{
                position: "absolute",
                top: "calc(100% + 4px)",
                left: 0,
                minWidth: 200,
                background: "var(--surface-card)",
                border: "1px solid var(--border-subtle)",
                borderRadius: 6,
                boxShadow: "0 4px 12px rgba(0,0,0,0.18)",
                padding: 6,
                zIndex: 10,
                fontSize: 12,
              }}
            >
              <div style={{ color: "var(--text-secondary)", padding: "2px 6px 4px", fontSize: 10, textTransform: "uppercase", letterSpacing: 0.4 }}>
                Bookmark scope
              </div>
              {(["thread", "stream", "global"] as BookmarkScope[]).map((scope) => {
                const active = bookmark.scopes!.includes(scope);
                const isDefault = bookmark.defaultScope === scope;
                return (
                  <div key={scope} style={{ display: "flex", alignItems: "center", gap: 4 }}>
                    <button
                      type="button"
                      data-testid={`page-nav-bookmark-scope-${scope}`}
                      onClick={() => bookmark.onToggleScope?.(scope)}
                      style={{
                        flex: 1,
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
                    <button
                      type="button"
                      title={isDefault ? "Default scope" : "Set as default scope"}
                      onClick={() => bookmark.onSetDefaultScope?.(scope)}
                      style={{
                        background: "transparent",
                        border: "none",
                        color: isDefault ? "var(--accent-fg)" : "var(--text-secondary)",
                        cursor: "pointer",
                        padding: "2px 6px",
                        fontSize: 10,
                      }}
                    >
                      {isDefault ? "● default" : "○ default"}
                    </button>
                  </div>
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
