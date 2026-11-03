import type { OpenWithProps } from "./pageMenuItems.js";
import type { MenuPosition } from "../menu.js";
import { PageModes } from "./PageModes.js";
import type { MutableRefObject, ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import type { AskTarget } from "../components/Prompts/askTarget.js";
import type { PagePin } from "../pages/PagePinMenu.js";
import { ActionButtons } from "../kit/ActionButtons.js";
import type { PageAction } from "../kit/actions.js";
import { PageMenu } from "./PageMenu.js";
import { barActions } from "./pageMenuItems.js";
import type { TabRef } from "./tabState.js";
import type { BookmarkScope } from "./bookmarks.js";

export interface PageNavBarProps {
  /** Back and forward through the tab's history. A pinned page has none
   *  (it is always itself), so its bar leaves them out. Default true. */
  history?: boolean;
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
  /** The page's name as text: what its Add to Context entry says. */
  pageLabel?: string;
  /** Drawn before the title: what the page is (an agent's logo). */
  titleIcon?: ReactNode;
  /** What kind of thing the tab shows ("Measure"), a muted chip before
   *  the title; none for a page route. */
  kind?: string | null;
  /** Optional bookmark affordance — when omitted, no star renders.
   *  The button always opens a popover that lets the user toggle this
   *  page's bookmark at one scope (thread / stream / project). */
  bookmark?: {
    /** The scope this page is bookmarked at; the star is filled when set. */
    scope: BookmarkScope | null;
    onToggleScope(scope: BookmarkScope): void;
    /** The same page is bookmarked at other values (its view): the star
     *  says so and the popover offers to move that bookmark here. */
    other?: { scope: BookmarkScope } | null;
    onUpdateOther?(): void;
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
  /** Optional self-contained comment navigator (count + prev/next jump
   *  + orphaned list) for comment-bearing pages. Rendered before the
   *  backlinks dropdown. */
  comments?: ReactNode;
  /** The page's actions, drawn here, never by the page: the primary and
   *  prominent ones as buttons, the rest in the ☰ menu. */
  actions?: readonly PageAction[];
  /** What the page would add to a dashboard: a Pin to Dashboard menu. */
  pin?: { tile: PagePin; onOpenPage(ref: TabRef): void; disabledReason?: string | null };
  /** The page's canonical ref (and stream): the Ask menu — Ask About
   *  This and the prompts about its kind (P6.D1/D2). Omitted, no Ask. */
  ask?: AskTarget;
  /** The pages that render the URI shown, when there are several: the
   *  bar's modes (`PageModes`) and the ☰'s Open With. */
  openWith?: OpenWithProps;
  /** The page's "See also" links, listed in the ☰. */
  seeAlso?: { links: ReadonlyArray<{ title: string; uri: string; description?: string | null }>; open(uri: string): void };
  /** The ☰'s opener, for the chrome's right-click on the body (`PageMenu`). */
  menuOpenAtRef?: MutableRefObject<((position: MenuPosition) => void) | null>;
}

/**
 * Browser-style navigation bar rendered inside `Page` chrome: how the page
 * is navigated (back/forward, its list, its title), its bookmark and
 * comments, its primary action, and one ☰ menu (`PageMenu`) with
 * everything else it can be acted on with. Kept dumb and props-driven —
 * the host wires it to the real navigation/bookmark/backlinks state.
 */
export function PageNavBar({
  history = true,
  canBack,
  canForward,
  onBack,
  onForward,
  siblings,
  title,
  titleIcon,
  kind = null,
  pageLabel,
  bookmark,
  backlinks,
  outbound,
  comments,
  actions,
  pin,
  ask,
  openWith,
  seeAlso,
  menuOpenAtRef,
}: PageNavBarProps) {
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
        // Panel-header tint (muted accent over the card surface) so the
        // page chrome reads as a header standing out from the content —
        // matches the rail/stream panel headers.
        background: "linear-gradient(var(--panel-header-bg), var(--panel-header-bg)), var(--surface-card)",
        boxShadow: "inset 0 -1px 0 rgba(0,0,0,0.18)",
        flexShrink: 0,
        minHeight: 36,
        position: "relative",
      }}
    >
      {history ? (
        <>
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
        </>
      ) : null}

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

      {title ? (
        <div
          style={{
            display: "flex",
            alignItems: titleIcon ? "center" : "baseline",
            gap: 8,
            flex: 1,
            minWidth: 0,
            paddingLeft: 6,
          }}
        >
          {titleIcon ? (
            <span data-testid="page-nav-title-icon" style={{ display: "inline-flex", flexShrink: 0, color: "var(--text-secondary)" }}>
              {titleIcon}
            </span>
          ) : null}
          {kind ? (
            <span data-testid="page-nav-kind" className="ox-badge" style={{ flexShrink: 0, color: "var(--text-secondary)" }}>
              {kind}
            </span>
          ) : null}
          <span
            data-testid="page-nav-title"
            title={typeof title === "string" ? title : undefined}
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
        </div>
      ) : (
        <div style={{ flex: 1 }} />
      )}

      {bookmark ? (
        <div style={{ position: "relative", display: "inline-flex" }}>
          <button
            type="button"
            data-testid="page-nav-bookmark"
            title={bookmark.scope == null && bookmark.other ? "Bookmarked at other values" : "Bookmark"}
            onClick={() => setScopeOpen((v) => !v)}
            aria-expanded={scopeOpen}
            style={{
              ...navButtonStyle(true),
              color: bookmark.scope != null || bookmark.other ? "var(--accent-fg)" : "var(--text-secondary)",
            }}
          >
            {bookmark.scope != null ? "★" : "☆"}
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
              {bookmark.scope == null && bookmark.other && bookmark.onUpdateOther ? (
                <button
                  type="button"
                  data-testid="page-nav-bookmark-update"
                  onClick={() => {
                    bookmark.onUpdateOther!();
                    setScopeOpen(false);
                  }}
                  style={{
                    display: "block",
                    width: "100%",
                    textAlign: "left",
                    padding: "4px 6px",
                    background: "transparent",
                    border: "none",
                    borderBottom: "1px solid var(--border-subtle)",
                    color: "var(--text-primary)",
                    cursor: "pointer",
                  }}
                >
                  Bookmarked at other values: update that bookmark
                </button>
              ) : null}
              <div style={{ padding: "4px 6px", fontSize: 11, color: "var(--muted)" }} data-testid="page-nav-bookmark-heading">
                Bookmark for:
              </div>
              {(["thread", "stream", "project"] as BookmarkScope[]).map((scope) => {
                const active = bookmark.scope === scope;
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
                    {scope === "thread" ? "This thread" : scope === "stream" ? "This stream" : "This project"}
                  </button>
                );
              })}
            </div>
          ) : null}
        </div>
      ) : null}

      {comments ?? null}

      {openWith ? <PageModes openWith={openWith} buttonStyle={navButtonStyle(true)} /> : null}
      {barActions(actions).length > 0 ? <ActionButtons actions={barActions(actions)} buttonStyle={navButtonStyle(true)} /> : null}
      <PageMenu
        actions={actions}
        ask={ask}
        label={pageLabel}
        openWith={openWith}
        seeAlso={seeAlso}
        backlinks={backlinks}
        outbound={outbound}
        pin={pin}
        openAtRef={menuOpenAtRef}
        // The ☰ is the bar's edge, not one more button: no box around it.
        buttonStyle={{ ...navButtonStyle(true), borderColor: "transparent" }}
      />
    </div>
  );
}

function navButtonStyle(enabled: boolean): React.CSSProperties {
  return {
    border: "1px solid var(--border-subtle)",
    // Transparent so the buttons sit on the header tint instead of
    // reading as darker boxes against it.
    background: "transparent",
    color: enabled ? "var(--text-primary)" : "var(--text-secondary)",
    padding: "4px 8px",
    borderRadius: 4,
    cursor: enabled ? "pointer" : "default",
    opacity: enabled ? 1 : 0.4,
    fontSize: "var(--text-sm)",
    minWidth: 28,
    // One height for every control in the bar, its glyph, icon or label
    // centered: a text button and an icon-only one line up.
    height: 28,
    boxSizing: "border-box",
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    lineHeight: 1,
  };
}
