import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { PageNavBar } from "./PageNavBar.js";
import { useOptionalPageNavigation } from "./PageNavigationContext.js";
import type { BookmarkScope } from "./bookmarks.js";

export interface PageNavBarConfig {
  canBack: boolean;
  canForward: boolean;
  onBack(): void;
  onForward(): void;
  siblings?: {
    prevLabel?: string;
    nextLabel?: string;
    onPrev?(): void;
    onNext?(): void;
    indicator?: string;
    indicatorTitle?: string;
    /** Full sibling list — when present, the indicator becomes a
     *  toggle that opens a dropdown of all entries. */
    entries?: Array<{ label: string }>;
    /** 0-based index of the active entry within `entries`. */
    activeIndex?: number;
    /** Jump to a specific sibling by index. */
    onSelect?(index: number): void;
  };
  bookmark?: {
    scopes: BookmarkScope[];
    onToggleScope(scope: BookmarkScope): void;
  };
  /** Backlinks dropdown content. Mutually exclusive with the
   *  legacy footer panel — when this is supplied, the footer is
   *  suppressed even if the `backlinks` prop is also set. */
  backlinks?: { count: number; body: ReactNode };
  /** Outbound dropdown — same shape as backlinks. Pages set this
   *  when they want to surface what they point AT in addition to
   *  what points at them. */
  outbound?: { count: number; body: ReactNode };
  /** Optional snapshots dropdown — same shape. FilePage uses this
   *  to expose the file's per-snapshot history. */
  snapshots?: { count: number; body: ReactNode };
  /** Optional comment navigator node (count + prev/next + orphaned
   *  list) for comment-bearing pages. */
  comments?: ReactNode;
  actions?: ReactNode;
}

export interface PageChip {
  label: string;
  /** Foreground color override (e.g. status color). */
  color?: string;
  /** Soft background color override. */
  background?: string;
  title?: string;
}

export interface PageProps {
  /** Page title shown in the header. Optional — when omitted, Page
   *  falls back to the title registered on `PageNavigationContext`
   *  via `usePageTitle`. Pages should prefer that programmatic path
   *  so the same string drives both the chrome header and the tab
   *  strip label. */
  title?: ReactNode;
  /** Optional kind/type label rendered as a small chip ("file", "tasks"…). */
  kind?: string;
  /** Optional status / metadata chips rendered next to the kind. */
  chips?: PageChip[];
  /** Optional kebab actions — rendered as `⋯` popover. */
  actions?: ReactNode;
  /** Page body. */
  children: ReactNode;
  /** Optional backlinks. When the active page has a nav bar (which it
   *  always does inside a `PageNavigationContext`), backlinks render
   *  as a dropdown in the nav bar with the count badge; otherwise the
   *  legacy collapsible footer renders. Pass either a bare `ReactNode`
   *  (count unknown) or `{ count, body }` to surface a badge. */
  backlinks?: ReactNode | { count: number; body: ReactNode };
  /** Optional outbound list — what THIS page points AT. Renders as a
   *  sibling dropdown next to Backlinks in the nav bar. Sibling to
   *  backlinks; same shape. */
  outbound?: { count: number; body: ReactNode };
  /** Optional snapshots list — for pages where per-snapshot
   *  history is meaningful (currently just FilePage). Renders as a
   *  sibling dropdown next to Backlinks/Outbound. */
  snapshots?: { count: number; body: ReactNode };
  /** Optional comment navigator node, rendered in the nav bar before
   *  Backlinks. Comment-bearing pages (file/wiki/task) pass a
   *  `<CommentNavigator targetKind targetId />`. */
  commentsNav?: ReactNode;
  /** Optional nav-bar config. When supplied, the browser-style nav bar
   *  renders between header and body, and (if it carries a `backlinks`
   *  block) suppresses the legacy footer panel. */
  navBar?: PageNavBarConfig;
  /** Test id applied to the page root. */
  testId?: string;
  /** When false, suppress the browser-style nav bar even if a
   *  PageNavigationContext is present. Defaults to true. The agent
   *  tab uses this to opt out; future bare-content pages can too. */
  showNavBar?: boolean;
  /** When false, suppress the title/chips/actions header. Defaults
   *  to true. */
  showHeader?: boolean;
  /** Body layout. `"full"` (default) renders edge-to-edge — today's
   *  behavior. `"details"` renders a two-column grid: a reading-width
   *  center column (capped at 760px) plus a sticky right rail
   *  (`rightRail`). Below ~960px body-container width the rail
   *  unmounts and the center column reflows. Purely responsive — no
   *  user-controlled toggle. */
  layout?: "full" | "details";
  /** Right-rail content for `layout="details"`. Ignored otherwise. */
  rightRail?: ReactNode;
}

/** Body-container width below which the right rail is unmounted. */
const DETAILS_RAIL_THRESHOLD_PX = 960;

/**
 * Shared chrome for every page rendered inside a tab body. Provides:
 *  - A header (title + kind chip + status/metadata chips + actions slot)
 *  - The page body
 *  - A collapsible Backlinks panel anchored at the bottom
 *
 * The chrome reads only semantic CSS variables. Both light and dark
 * themes are styled by `public/index.html`.
 */
export function Page({ title, kind, chips, actions, children, backlinks, outbound, snapshots, commentsNav, navBar, testId, showNavBar = true, showHeader = true, layout = "full", rightRail }: PageProps) {
  const [backlinksOpen, setBacklinksOpen] = useState(false);
  // Pages that don't pass an explicit `navBar` prop still get one
  // when rendered inside a PageNavigationContext provider — that's
  // how the host (App.tsx) injects browser-style back/forward into
  // every page without requiring each page module to wire it.
  const ctxNav = useOptionalPageNavigation();
  // Normalize the `backlinks` prop: `{ count, body }` shape carries an
  // explicit count for the dropdown badge; a bare ReactNode means the
  // count is unknown (label shows just "Backlinks").
  const backlinksHasCount =
    backlinks !== undefined &&
    backlinks !== null &&
    typeof backlinks === "object" &&
    !Array.isArray(backlinks) &&
    !("$$typeof" in backlinks) &&
    "body" in (backlinks as object) &&
    "count" in (backlinks as object);
  const backlinksBody: ReactNode | undefined = backlinks === undefined
    ? undefined
    : backlinksHasCount
      ? (backlinks as { count: number; body: ReactNode }).body
      : (backlinks as ReactNode);
  const backlinksCount: number | undefined = backlinksHasCount
    ? (backlinks as { count: number; body: ReactNode }).count
    : undefined;
  const effectiveTitle: ReactNode = title ?? ctxNav?.title ?? "";
  const baseNavBar: PageNavBarConfig | undefined = !showNavBar ? undefined : navBar ?? (ctxNav ? {
    canBack: ctxNav.canGoBack,
    canForward: ctxNav.canGoForward,
    onBack: ctxNav.goBack,
    onForward: ctxNav.goForward,
    siblings: ctxNav.siblings
      ? {
          prevLabel: ctxNav.siblings.entries[ctxNav.siblings.index - 1]?.label,
          nextLabel: ctxNav.siblings.entries[ctxNav.siblings.index + 1]?.label,
          onPrev: ctxNav.goPrevSibling,
          onNext: ctxNav.goNextSibling,
          indicator: `${ctxNav.siblings.index + 1} of ${ctxNav.siblings.entries.length}`,
          indicatorTitle: ctxNav.siblings.title,
          entries: ctxNav.siblings.entries.map((e) => ({ label: e.label })),
          activeIndex: ctxNav.siblings.index,
          onSelect: ctxNav.goSibling,
        }
      : undefined,
    bookmark: ctxNav.bookmark
      ? {
          scopes: ctxNav.bookmark.scopes,
          onToggleScope: (scope) => ctxNav.bookmark!.toggle(scope),
        }
      : undefined,
  } : undefined);
  // When a nav bar is present, promote backlinks into its dropdown
  // and suppress the legacy footer. Pages that explicitly set
  // `navBar.backlinks` themselves still win.
  const effectiveNavBar: PageNavBarConfig | undefined = baseNavBar
    ? {
        ...baseNavBar,
        backlinks: baseNavBar.backlinks
          ?? (backlinksBody !== undefined
            ? { count: backlinksCount ?? 0, body: backlinksBody }
            : undefined),
        outbound: baseNavBar.outbound ?? outbound,
        snapshots: baseNavBar.snapshots ?? snapshots,
        comments: baseNavBar.comments ?? commentsNav,
      }
    : undefined;
  const navBarOwnsBacklinks = effectiveNavBar?.backlinks !== undefined;

  return (
    <div
      data-testid={testId ?? "page"}
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
        background: "var(--surface-card)",
        color: "var(--text-primary)",
      }}
    >
      {effectiveNavBar ? (
        <PageNavBar
          canBack={effectiveNavBar.canBack}
          canForward={effectiveNavBar.canForward}
          onBack={effectiveNavBar.onBack}
          onForward={effectiveNavBar.onForward}
          siblings={effectiveNavBar.siblings}
          title={effectiveTitle}
          kind={kind}
          bookmark={effectiveNavBar.bookmark}
          backlinks={effectiveNavBar.backlinks}
          outbound={effectiveNavBar.outbound}
          snapshots={effectiveNavBar.snapshots}
          comments={effectiveNavBar.comments}
          actions={effectiveNavBar.actions}
        />
      ) : null}
      {showHeader && !effectiveNavBar ? (
      <header
        data-testid="page-header"
        style={{
          padding: "14px 20px",
          borderBottom: "1px solid var(--border-subtle)",
          display: "flex",
          alignItems: "center",
          gap: 12,
          flexShrink: 0,
          minHeight: 56,
        }}
      >
        <div style={{ display: "flex", alignItems: "baseline", gap: 10, flex: 1, minWidth: 0 }}>
          <span
            data-testid="page-title"
            style={{
              fontSize: "var(--text-lg)",
              fontWeight: 600,
              color: "var(--text-primary)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {effectiveTitle}
          </span>
          {kind ? (
            <span
              data-testid="page-kind"
              style={{
                fontSize: 11,
                fontWeight: "var(--weight-medium)",
                color: "var(--text-secondary)",
                background: "var(--surface-tab-inactive)",
                padding: "2px 6px",
                borderRadius: 4,
                textTransform: "lowercase",
              }}
            >
              {kind}
            </span>
          ) : null}
          {chips?.map((chip, i) => (
            <span
              key={i}
              title={chip.title}
              style={{
                fontSize: 11,
                fontWeight: "var(--weight-medium)",
                color: chip.color ?? "var(--text-secondary)",
                background: chip.background ?? "transparent",
                padding: "2px 6px",
                borderRadius: 4,
                border: chip.background ? "none" : "1px solid var(--border-subtle)",
              }}
            >
              {chip.label}
            </span>
          ))}
        </div>
        {actions ? (
          <div style={{ display: "flex", gap: 6, flexShrink: 0 }}>{actions}</div>
        ) : null}
      </header>
      ) : null}
      {showHeader && effectiveNavBar && ((chips && chips.length > 0) || actions) ? (
        <div
          data-testid="page-chips"
          style={{
            padding: "6px 20px",
            borderBottom: "1px solid var(--border-subtle)",
            display: "flex",
            alignItems: "center",
            gap: 8,
            flexShrink: 0,
            minHeight: 32,
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: 8, flex: 1, minWidth: 0, flexWrap: "wrap" }}>
            {chips?.map((chip, i) => (
              <span
                key={i}
                title={chip.title}
                style={{
                  fontSize: 11,
                  fontWeight: "var(--weight-medium)",
                  color: chip.color ?? "var(--text-secondary)",
                  background: chip.background ?? "transparent",
                  padding: "2px 6px",
                  borderRadius: 4,
                  border: chip.background ? "none" : "1px solid var(--border-subtle)",
                }}
              >
                {chip.label}
              </span>
            ))}
          </div>
          {actions ? (
            <div style={{ display: "flex", gap: 6, flexShrink: 0 }}>{actions}</div>
          ) : null}
        </div>
      ) : null}
      {layout === "details" ? (
        <DetailsBody rightRail={rightRail}>{children}</DetailsBody>
      ) : (
        <div style={{ flex: 1, minHeight: 0, minWidth: 0, overflow: "auto", display: "flex", flexDirection: "column" }}>
          {children}
        </div>
      )}
      {backlinksBody !== undefined && !navBarOwnsBacklinks ? (
        <div
          data-testid="page-backlinks"
          style={{
            borderTop: "1px solid var(--border-subtle)",
            background: "var(--surface-app)",
            flexShrink: 0,
          }}
        >
          <button
            type="button"
            data-testid="page-backlinks-toggle"
            onClick={() => setBacklinksOpen((v) => !v)}
            style={{
              border: "none",
              background: "transparent",
              color: "var(--text-secondary)",
              padding: "8px 16px",
              cursor: "pointer",
              fontSize: "var(--text-xs)",
              width: "100%",
              textAlign: "left",
            }}
            aria-expanded={backlinksOpen}
          >
            {backlinksOpen ? "▾" : "▸"} Backlinks
          </button>
          {backlinksOpen ? (
            <div data-testid="page-backlinks-body" style={{ padding: "0 16px 12px", fontSize: "var(--text-xs)" }}>
              {backlinksBody}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function DetailsBody({ children, rightRail }: { children: ReactNode; rightRail?: ReactNode }) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [showRail, setShowRail] = useState(true);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver((entries) => {
      const width = entries[0]?.contentRect.width ?? el.clientWidth;
      setShowRail(width >= DETAILS_RAIL_THRESHOLD_PX);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const railVisible = showRail && rightRail !== undefined && rightRail !== null;

  return (
    <div
      ref={scrollRef}
      data-testid="page-details-body"
      style={{
        flex: 1,
        minHeight: 0,
        minWidth: 0,
        overflow: "auto",
        display: "grid",
        gridTemplateColumns: railVisible ? "1fr 320px" : "1fr",
        gap: railVisible ? 24 : 0,
        padding: 24,
        alignItems: "start",
      }}
    >
      <div
        data-testid="page-details-center"
        style={{
          width: "100%",
          minWidth: 0,
        }}
      >
        {children}
      </div>
      {railVisible ? (
        <aside
          data-testid="page-details-rail"
          style={{
            position: "sticky",
            top: 0,
            alignSelf: "start",
            maxHeight: "calc(100vh - 48px)",
            overflow: "auto",
            minWidth: 0,
          }}
        >
          {rightRail}
        </aside>
      ) : null}
    </div>
  );
}
