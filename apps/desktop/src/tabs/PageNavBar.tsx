import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { PageKindIcon, pageKindLabel } from "../pageKinds.js";
import type { BookmarkScope } from "./bookmarks.js";

export interface PageNavBarProps {
  canBack: boolean;
  canForward: boolean;
  onBack(): void;
  onForward(): void;
  /** Sibling-list navigation. When omitted, the up/down buttons are
   *  hidden — the page wasn't opened from a list. When supplied,
   *  buttons are disabled at edges and the hover-title shows the
   *  label of the prev/next entry. */
  siblings?: {
    prevLabel?: string;
    nextLabel?: string;
    onPrev?(): void;
    onNext?(): void;
    /** "3 of 12" indicator rendered between the buttons. */
    indicator?: string;
    /** Hover-tooltip on the indicator describing the originating
     *  list (e.g. "Recently modified", "Backlinks"). */
    indicatorTitle?: string;
    /** Full sibling list — when present, the indicator becomes a
     *  toggle that opens a dropdown listing every entry, mirroring
     *  the CenterTabs overflow ▾ pattern so the user can jump to a
     *  sibling instead of stepping through them. */
    entries?: Array<{ label: string }>;
    /** 0-based index of the active entry within `entries`. */
    activeIndex?: number;
    /** Jump to a sibling by index. */
    onSelect?(index: number): void;
  };
  /** Page title rendered to the right of the back/forward arrows. */
  title?: ReactNode;
  /** Small kind chip rendered after the title ("wiki", "file", …). */
  kind?: string;
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
  /** Optional outbound dropdown — same shape as `backlinks`. Sits
   *  next to the backlinks button so a user can see both directions
   *  of the page-ref graph from the chrome. */
  outbound?: {
    count: number;
    body: ReactNode;
  };
  /** Optional snapshots dropdown — same shape, used by FilePage to
   *  expose this file's per-snapshot history without crowding the
   *  outbound (page-ref) panel. */
  snapshots?: {
    count: number;
    body: ReactNode;
  };
  /** Optional self-contained comment navigator (count + prev/next jump
   *  + orphaned list) for comment-bearing pages. Rendered before the
   *  backlinks dropdown. */
  comments?: ReactNode;
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
  siblings,
  title,
  kind,
  bookmark,
  backlinks,
  outbound,
  snapshots,
  comments,
  actions,
}: PageNavBarProps) {
  const [backlinksOpen, setBacklinksOpen] = useState(false);
  const [outboundOpen, setOutboundOpen] = useState(false);
  const [snapshotsOpen, setSnapshotsOpen] = useState(false);
  const [scopeOpen, setScopeOpen] = useState(false);
  const [siblingListOpen, setSiblingListOpen] = useState(false);
  const siblingPopoverRef = useRef<HTMLDivElement | null>(null);
  const siblingToggleRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    if (!siblingListOpen) return;
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node;
      if (siblingPopoverRef.current?.contains(target)) return;
      if (siblingToggleRef.current?.contains(target)) return;
      setSiblingListOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setSiblingListOpen(false);
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [siblingListOpen]);

  return (
    <div
      data-testid="page-nav-bar"
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "6px 12px",
        borderBottom: "1px solid var(--border-strong, var(--border-subtle))",
        background: "var(--surface-rail, var(--surface-app))",
        boxShadow: "inset 0 -1px 0 rgba(0,0,0,0.18)",
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

      {siblings ? (
        <div
          data-testid="page-nav-siblings"
          style={{ display: "inline-flex", alignItems: "center", gap: 4, marginLeft: 4 }}
        >
          <button
            type="button"
            data-testid="page-nav-sibling-prev"
            title={siblings.prevLabel ? `Previous: ${siblings.prevLabel}` : "Previous in list"}
            disabled={!siblings.onPrev}
            onClick={siblings.onPrev}
            style={navButtonStyle(!!siblings.onPrev)}
          >
            ↑
          </button>
          <button
            type="button"
            data-testid="page-nav-sibling-next"
            title={siblings.nextLabel ? `Next: ${siblings.nextLabel}` : "Next in list"}
            disabled={!siblings.onNext}
            onClick={siblings.onNext}
            style={navButtonStyle(!!siblings.onNext)}
          >
            ↓
          </button>
          {siblings.indicator ? (
            siblings.entries && siblings.entries.length > 0 && siblings.onSelect ? (
              <div style={{ position: "relative", display: "inline-flex" }}>
                <button
                  ref={siblingToggleRef}
                  type="button"
                  data-testid="page-nav-sibling-indicator"
                  title={siblings.indicatorTitle ?? "Show all"}
                  aria-expanded={siblingListOpen}
                  aria-haspopup="listbox"
                  onClick={() => setSiblingListOpen((v) => !v)}
                  style={{
                    border: "none",
                    background: "transparent",
                    padding: "2px 4px",
                    marginLeft: 2,
                    fontSize: 11,
                    color: "var(--text-secondary)",
                    fontVariantNumeric: "tabular-nums",
                    cursor: "pointer",
                    borderRadius: 3,
                  }}
                >
                  {siblings.indicator} ▾
                </button>
                {siblingListOpen ? (
                  <div
                    ref={siblingPopoverRef}
                    data-testid="page-nav-sibling-list"
                    role="listbox"
                    style={{
                      position: "absolute",
                      top: "calc(100% + 4px)",
                      left: 0,
                      minWidth: 240,
                      maxWidth: 480,
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
                    {siblings.indicatorTitle ? (
                      <div
                        style={{
                          padding: "4px 8px 6px",
                          fontSize: 10,
                          textTransform: "uppercase",
                          letterSpacing: 0.4,
                          color: "var(--text-secondary)",
                        }}
                      >
                        {siblings.indicatorTitle}
                      </div>
                    ) : null}
                    {siblings.entries.map((entry, i) => {
                      const active = i === siblings.activeIndex;
                      return (
                        <button
                          key={i}
                          type="button"
                          data-testid={`page-nav-sibling-list-item-${i}`}
                          role="option"
                          aria-selected={active}
                          onClick={() => {
                            setSiblingListOpen(false);
                            if (!active) siblings.onSelect?.(i);
                          }}
                          style={{
                            display: "flex",
                            alignItems: "center",
                            gap: 6,
                            width: "100%",
                            textAlign: "left",
                            padding: "4px 8px",
                            background: active ? "var(--surface-tab-active, var(--surface-rail))" : "transparent",
                            border: "none",
                            color: "var(--text-primary)",
                            cursor: "pointer",
                            borderRadius: 4,
                            fontVariantNumeric: "tabular-nums",
                          }}
                        >
                          <span
                            style={{
                              flexShrink: 0,
                              width: 28,
                              color: "var(--text-secondary)",
                              fontSize: 11,
                              textAlign: "right",
                            }}
                          >
                            {i + 1}
                          </span>
                          <span
                            style={{
                              overflow: "hidden",
                              textOverflow: "ellipsis",
                              whiteSpace: "nowrap",
                              fontWeight: active ? 600 : 400,
                            }}
                          >
                            {entry.label}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                ) : null}
              </div>
            ) : (
              <span
                data-testid="page-nav-sibling-indicator"
                title={siblings.indicatorTitle}
                style={{
                  fontSize: 11,
                  color: "var(--text-secondary)",
                  marginLeft: 2,
                  fontVariantNumeric: "tabular-nums",
                  cursor: siblings.indicatorTitle ? "help" : "default",
                }}
              >
                {siblings.indicator}
              </span>
            )
          ) : null}
        </div>
      ) : null}

      {title || kind ? (
        <div
          style={{
            display: "flex",
            alignItems: "baseline",
            gap: 8,
            flex: 1,
            minWidth: 0,
            paddingLeft: 6,
          }}
        >
          {title ? (
            <span
              data-testid="page-nav-title"
              style={{
                fontSize: "var(--text-base)",
                fontWeight: 600,
                color: "var(--text-primary)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {title}
            </span>
          ) : null}
          {kind ? (
            <span
              data-testid="page-nav-kind"
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: 4,
                fontSize: 11,
                fontWeight: "var(--weight-medium)",
                color: "var(--text-secondary)",
                background: "var(--surface-tab-inactive)",
                padding: "2px 6px",
                borderRadius: 4,
                textTransform: "lowercase",
                flexShrink: 0,
              }}
            >
              <PageKindIcon kind={kind} size={12} />
              {pageKindLabel(kind)}
            </span>
          ) : null}
        </div>
      ) : (
        <div style={{ flex: 1 }} />
      )}

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
                right: 0,
                minWidth: 180,
                background: "var(--surface-card)",
                border: "1px solid var(--border-subtle)",
                borderRadius: 6,
                boxShadow: "0 4px 12px rgba(0,0,0,0.18)",
                padding: 6,
                zIndex: 10,
                fontSize: "var(--text-xs)",
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

      {comments ?? null}

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
              fontSize: "var(--text-xs)",
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
                right: 0,
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

      {outbound ? (
        <div style={{ position: "relative" }}>
          <button
            type="button"
            data-testid="page-nav-outbound-toggle"
            onClick={() => setOutboundOpen((v) => !v)}
            disabled={outbound.count === 0}
            aria-expanded={outboundOpen}
            style={{
              ...navButtonStyle(outbound.count > 0),
              padding: "4px 10px",
              fontSize: "var(--text-xs)",
            }}
          >
            Outbound ({outbound.count}) {outboundOpen ? "▾" : "▸"}
          </button>
          {outboundOpen && outbound.count > 0 ? (
            <div
              data-testid="page-nav-outbound-popover"
              style={{
                position: "absolute",
                top: "calc(100% + 4px)",
                right: 0,
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
              {outbound.body}
            </div>
          ) : null}
        </div>
      ) : null}

      {snapshots ? (
        <div style={{ position: "relative" }}>
          <button
            type="button"
            data-testid="page-nav-snapshots-toggle"
            onClick={() => setSnapshotsOpen((v) => !v)}
            disabled={snapshots.count === 0}
            aria-expanded={snapshotsOpen}
            style={{
              ...navButtonStyle(snapshots.count > 0),
              padding: "4px 10px",
              fontSize: "var(--text-xs)",
            }}
          >
            Snapshots ({snapshots.count}) {snapshotsOpen ? "▾" : "▸"}
          </button>
          {snapshotsOpen && snapshots.count > 0 ? (
            <div
              data-testid="page-nav-snapshots-popover"
              style={{
                position: "absolute",
                top: "calc(100% + 4px)",
                right: 0,
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
              {snapshots.body}
            </div>
          ) : null}
        </div>
      ) : null}

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
    fontSize: "var(--text-sm)",
    minWidth: 28,
  };
}
