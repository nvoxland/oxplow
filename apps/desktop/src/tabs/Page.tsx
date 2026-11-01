import type { CSSProperties, ReactNode } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { openWithOf } from "../pages/renderers.js";
import { PageNavBar } from "./PageNavBar.js";
import { TabModalHostContext } from "../components/TabModal.js";
import { useOptionalPageNavigation } from "./PageNavigationContext.js";
import { useResources } from "../resources.js";
import { typeOf } from "../uri/uri.js";
import type { BookmarkScope } from "./bookmarks.js";
import type { AskTarget } from "../components/Prompts/askTarget.js";
import { chipsFor } from "../components/decorators.js";
import { useDecorations } from "../components/useDecorations.js";
import type { PagePin } from "../pages/PagePinMenu.js";
import { ActionButtons } from "../kit/ActionButtons.js";
import type { PageAction } from "../kit/actions.js";
import type { TabRef } from "./tabState.js";

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
    scope: BookmarkScope | null;
    onToggleScope(scope: BookmarkScope): void;
    other?: { scope: BookmarkScope } | null;
    onUpdateOther?(): void;
  };
  /** The page's ref and stream, for the Ask menu. */
  ask?: AskTarget;
  /** Backlinks dropdown content. Mutually exclusive with the
   *  collapsible footer panel — when this is supplied, the footer is
   *  suppressed even if the `backlinks` prop is also set. */
  backlinks?: { count: number; body: ReactNode };
  /** Outbound dropdown — same shape as backlinks. Pages set this
   *  when they want to surface what they point AT in addition to
   *  what points at them. */
  outbound?: { count: number; body: ReactNode };
  /** Optional comment navigator node (count + prev/next + orphaned
   *  list) for comment-bearing pages. */
  comments?: ReactNode;
  /** The pages that render the URI shown, when there are several. */
  openWith?: import("./pageMenuItems.js").OpenWithProps;
}

export interface PageChip {
  label: string;
  /** Foreground color override (e.g. status color). */
  color?: string;
  /** Soft background color override. */
  background?: string;
  title?: string;
  /** Pressing the chip (a wiki page's freshness opens its report). */
  onClick?(): void;
}

export interface PageProps {
  /** Page title shown in the header. Optional — when omitted, Page
   *  falls back to the title registered on `PageNavigationContext`
   *  via `usePageTitle`. Pages should prefer that programmatic path
   *  so the same string drives both the chrome header and the tab
   *  strip label. */
  title?: ReactNode;
  /** Optional status / metadata chips rendered next to the title. */
  chips?: PageChip[];
  /** What the page can do, as data: core draws them in the nav bar (in
   *  the header when the page shows no bar). */
  actions?: readonly PageAction[];
  /** What the page would add to a dashboard: the nav bar's Pin to
   *  Dashboard. */
  pin?: { tile: PagePin; onOpenPage(ref: TabRef): void };
  /** The page's "See also" links: a submenu of the ☰, and how one opens. */
  seeAlso?: { links: ReadonlyArray<{ title: string; uri: string; description?: string | null }>; open(uri: string): void };
  /** Page body. */
  children: ReactNode;
  /** Optional backlinks. When the active page has a nav bar (which it
   *  always does inside a `PageNavigationContext`), backlinks render
   *  as a dropdown in the nav bar with the count badge; otherwise the
   *  collapsible footer renders. Pass either a bare `ReactNode`
   *  (count unknown) or `{ count, body }` to surface a badge. */
  backlinks?: ReactNode | { count: number; body: ReactNode };
  /** Optional outbound list — what THIS page points AT. Renders as a
   *  sibling dropdown next to Backlinks in the nav bar. Sibling to
   *  backlinks; same shape. */
  outbound?: { count: number; body: ReactNode };
  /** Optional comment navigator node, rendered in the nav bar before
   *  Backlinks. Comment-bearing pages (file/wiki/task) pass a
   *  `<CommentNavigator targetKind targetId />`. */
  commentsNav?: ReactNode;
  /** Optional nav-bar config. When supplied, the browser-style nav bar
   *  renders between header and body, and (if it carries a `backlinks`
   *  block) suppresses the footer panel. */
  navBar?: PageNavBarConfig;
  /** Test id applied to the page root. */
  testId?: string;
  /** Drawn before the title in the nav bar: what the page is (an agent
   *  session's harness logo). */
  titleIcon?: ReactNode;
  /** The body's width: `"full"` (default) is every pixel, and the page
   *  owns its padding; `"reading"` is a padded, bounded reading column
   *  (`.oxplow-reading-column`, 78ch, centered) that its children fill. */
  width?: "full" | "reading";
  /** The body is one region that takes the page's whole height (a file, a
   *  diff): beside the rail it stretches instead of sitting at the top,
   *  and its children lay out as a column. Defaults to false. */
  fill?: boolean;
  /** The page's control panel, a sticky column at the right, beside a
   *  body of either width (padded then). Below ~960px it moves to the top
   *  of the body. Purely responsive — no user-controlled toggle. */
  rightRail?: ReactNode;
  /** Title for the right rail's panel header. Defaults to "Details". */
  rightRailTitle?: ReactNode;
}

/** Body-container width below which the right rail moves inline (it never
 *  disappears — `RailBody` relocates the same panel to the top of the
 *  center column). */
const DETAILS_RAIL_THRESHOLD_PX = 960;

/**
 * Shared chrome for every page rendered inside a tab body. Provides:
 *  - A header (title + status/metadata chips + actions slot); the nav
 *    bar names the kind of thing the tab shows (`kindOfTab`)
 *  - The page body
 *  - A collapsible Backlinks panel anchored at the bottom
 *
 * The chrome reads only semantic CSS variables. Both light and dark
 * themes are styled by `public/index.html`.
 */
export function Page({ title, titleIcon, chips: ownChips, actions, pin, seeAlso, children, backlinks, outbound, commentsNav, navBar, testId, width = "full", fill = false, rightRail, rightRailTitle }: PageProps) {
  const [backlinksOpen, setBacklinksOpen] = useState(false);
  // Pages that don't pass an explicit `navBar` prop still get one
  // when rendered inside a PageNavigationContext provider — that's
  // how the host (App.tsx) injects browser-style back/forward into
  // every page without requiring each page module to wire it.
  const ctxNav = useOptionalPageNavigation();
  const resources = useResources();
  const kind = kindOfTab(ctxNav?.ref?.id ?? null, resources);
  // Extensions' decorations for this page's ref (P6b.C5), after the page's
  // own chips: additive, the header is complete without them.
  const pageRef = ctxNav?.ask?.ref ?? null;
  const decorations = useDecorations("ref-chip", pageRef ? [pageRef] : []);
  const decorationChips = pageRef ? chipsFor(decorations, pageRef) : [];
  const chips = decorationChips.length > 0 ? [...(ownChips ?? []), ...decorationChips] : ownChips;
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
  // The pages that render this URI, to pick among (the daemon's list).
  const shownRenderers = ctxNav?.renderers;
  const shownView = ctxNav?.viewParams;
  const patchView = ctxNav?.patchViewParams;
  const openWith = useMemo(() => openWithOf(shownRenderers, shownView, patchView), [shownRenderers, shownView, patchView]);
  const pinned = ctxNav?.pinned ?? false;
  const baseNavBar: PageNavBarConfig | undefined = navBar ?? (ctxNav ? {
    canBack: ctxNav.canGoBack,
    canForward: ctxNav.canGoForward,
    onBack: ctxNav.goBack,
    onForward: ctxNav.goForward,
    siblings: ctxNav.origin
      ? {
          prevLabel: ctxNav.origin.items[ctxNav.origin.index - 1]?.label,
          nextLabel: ctxNav.origin.items[ctxNav.origin.index + 1]?.label,
          onPrev: ctxNav.origin.index > 0 ? () => ctxNav.stepOrigin?.(ctxNav.origin!.index - 1) : undefined,
          onNext: ctxNav.origin.index < ctxNav.origin.items.length - 1 ? () => ctxNav.stepOrigin?.(ctxNav.origin!.index + 1) : undefined,
          indicator: `${ctxNav.origin.index + 1} of ${ctxNav.origin.items.length}`,
          indicatorTitle: ctxNav.origin.title,
          entries: ctxNav.origin.items.map((e) => ({ label: e.label })),
          activeIndex: ctxNav.origin.index,
          onSelect: ctxNav.stepOrigin,
        }
      : undefined,
    bookmark: ctxNav.bookmark
      ? {
          scope: ctxNav.bookmark.scope,
          onToggleScope: (scope) => ctxNav.bookmark!.toggle(scope),
          other: ctxNav.bookmark.other,
          onUpdateOther: ctxNav.bookmark.updateOther,
        }
      : undefined,
    ask: ctxNav.ask,
    openWith,
  } : undefined);
  // When a nav bar is present, promote backlinks into its dropdown
  // and suppress the footer. Pages that explicitly set
  // `navBar.backlinks` themselves still win.
  const effectiveNavBar: PageNavBarConfig | undefined = baseNavBar
    ? {
        ...baseNavBar,
        backlinks: baseNavBar.backlinks
          ?? (backlinksBody !== undefined
            ? { count: backlinksCount ?? 0, body: backlinksBody }
            : undefined),
        outbound: baseNavBar.outbound ?? outbound,
        comments: baseNavBar.comments ?? commentsNav,
      }
    : undefined;
  const [modalHost, setModalHost] = useState<HTMLDivElement | null>(null);
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
          history={!pinned}
          canBack={effectiveNavBar.canBack}
          canForward={effectiveNavBar.canForward}
          onBack={effectiveNavBar.onBack}
          onForward={effectiveNavBar.onForward}
          siblings={effectiveNavBar.siblings}
          title={effectiveTitle}
          titleIcon={titleIcon}
          kind={kind}
          pageLabel={typeof effectiveTitle === "string" ? effectiveTitle : undefined}
          bookmark={effectiveNavBar.bookmark}
          backlinks={effectiveNavBar.backlinks}
          outbound={effectiveNavBar.outbound}
          comments={effectiveNavBar.comments}
          actions={actions}
          pin={pin}
          ask={effectiveNavBar.ask}
          openWith={effectiveNavBar.openWith}
          seeAlso={seeAlso}
        />
      ) : null}
      {!effectiveNavBar ? (
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
            title={typeof effectiveTitle === "string" ? effectiveTitle : undefined}
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
          {chips?.map((chip, i) => <Chip key={i} chip={chip} />)}
        </div>
        {actions && actions.length > 0 ? <ActionButtons actions={actions} /> : null}
      </header>
      ) : null}
      {effectiveNavBar && chips && chips.length > 0 ? (
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
            {chips.map((chip, i) => <Chip key={i} chip={chip} />)}
          </div>
        </div>
      ) : null}
      {/* The area under the nav bar: the body, and where a tab's dialog docks. */}
      <div style={{ flex: 1, minHeight: 0, minWidth: 0, display: "flex", flexDirection: "column", position: "relative" }}>
        <TabModalHostContext.Provider value={modalHost}>
          {width === "reading" || rightRail != null ? (
            <RailBody width={width} fill={fill} rightRail={rightRail} rightRailTitle={rightRailTitle}>{children}</RailBody>
          ) : (
            <div data-testid="page-body" style={{ flex: 1, minHeight: 0, minWidth: 0, overflow: "auto", display: "flex", flexDirection: "column", padding: PAGE_GUTTER_PX }}>
              {children}
            </div>
          )}
        </TabModalHostContext.Provider>
        <div ref={setModalHost} data-testid="tab-modal-host" style={{ position: "absolute", inset: 0, zIndex: 20, pointerEvents: "none" }} />
      </div>
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

/** A page's status or metadata chip, pressable when it says what it opens. */
function Chip({ chip }: { chip: PageChip }) {
  const style: CSSProperties = {
    fontSize: "var(--text-xs)",
    fontWeight: "var(--weight-medium)",
    color: chip.color ?? "var(--text-secondary)",
    background: chip.background ?? "transparent",
    padding: "2px 6px",
    borderRadius: "var(--radius-sm)",
    border: chip.background ? "none" : "1px solid var(--border-subtle)",
  };
  return chip.onClick ? (
    <button type="button" title={chip.title} onClick={chip.onClick} style={{ ...style, font: "inherit", fontSize: style.fontSize, cursor: "pointer" }}>
      {chip.label}
    </button>
  ) : (
    <span title={chip.title} style={style}>
      {chip.label}
    </span>
  );
}

/** The gutter around a page's body, with a rail or without. */
const PAGE_GUTTER_PX = 24;

/** A padded body: a reading column or every pixel, with the page's rail
 *  beside it (or above it, when narrow). */
function RailBody({ children, width, fill, rightRail, rightRailTitle }: { children: ReactNode; width: "full" | "reading"; fill: boolean; rightRail?: ReactNode; rightRailTitle?: ReactNode }) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [showRail, setShowRail] = useState(true);
  // How tall the body shows (inside its padding): the side rail scrolls
  // within it. The window's height would count the app's chrome above.
  const [shown, setShown] = useState<number | null>(null);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver((entries) => {
      const width = entries[0]?.contentRect.width ?? el.clientWidth;
      setShowRail(width >= DETAILS_RAIL_THRESHOLD_PX);
      setShown(entries[0]?.contentRect.height ?? null);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const railProvided = rightRail !== undefined && rightRail !== null;
  const railAside = showRail && railProvided;
  // Narrow + has rail content → keep the SAME panel, just stacked at the top
  // of the center column instead of the side. The rail never just vanishes.
  const railInline = !showRail && railProvided;

  // The rail panel — identical look (card surface + tinted header band)
  // whether it sits in the side rail (`aside`, sticky) or stacked inline at
  // the top of the center column when the rail collapses.
  const railPanel = (variant: "aside" | "inline") => (
    <aside
      data-testid="page-rail"
      style={{
        ...(variant === "aside"
          ? {
              position: "sticky",
              top: 0,
              alignSelf: "start",
              maxHeight: shown === null ? undefined : `${shown}px`,
              overflow: "auto",
            }
          : fill
            ? // Stacked above a body that fills the page: a capped panel
              // that scrolls, so the body keeps the rest of the height.
              { marginBottom: 24, flex: "none", maxHeight: "35vh", overflow: "auto" }
            : { marginBottom: 24 }),
        minWidth: 0,
        background: "var(--surface-card)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 8,
        overflowX: "hidden",
      }}
    >
      <div
        data-testid="page-rail-title"
        style={{
          display: "flex",
          // The title keeps its line; the actions wrap below it when they
          // don't fit beside it (tsk1043: they overlapped it).
          flexWrap: "wrap",
          alignItems: "center",
          gap: 8,
          padding: "8px 10px",
          // Opaque: the rail's items scroll under it.
          background: "var(--surface-chrome)",
          borderBottom: "1px solid var(--border-subtle)",
          fontSize: 11,
          fontWeight: 600,
          color: "var(--text-secondary)",
          textTransform: "uppercase",
          letterSpacing: 0.4,
          ...(variant === "aside" ? { position: "sticky", top: 0, zIndex: 1 } : {}),
        }}
      >
        <span style={{ flex: "1 0 auto" }}>{rightRailTitle ?? "Details"}</span>
      </div>
      <div style={{ padding: 12 }}>{rightRail}</div>
    </aside>
  );

  return (
    <div
      ref={scrollRef}
      data-testid="page-rail-body"
      style={{
        flex: 1,
        minHeight: 0,
        minWidth: 0,
        overflow: "auto",
        display: "grid",
        gridTemplateColumns: railAside ? "1fr 320px" : "1fr",
        gap: railAside ? 24 : 0,
        padding: PAGE_GUTTER_PX,
        // A body that fills stretches to the page's height, row and all.
        alignItems: fill ? "stretch" : "start",
        gridTemplateRows: fill ? "minmax(0, 1fr)" : undefined,
      }}
    >
      <div
        data-testid="page-rail-center"
        // `oxplow-reading-column` (index.html) owns the reading-width cap
        // + centering and cancels child self-width, so the title, body,
        // and activity all live within one bounded, centered column
        // instead of each managing its own width. `minWidth: 0` keeps the
        // grid item from overflowing the track.
        className={width === "reading" ? "oxplow-reading-column" : undefined}
        style={fill ? { minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column" } : { minWidth: 0 }}
      >
        {railInline ? railPanel("inline") : null}
        {children}
      </div>
      {railAside ? railPanel("aside") : null}
    </div>
  );
}

/** What kind of thing a tab shows: its URI's resource's label ("Measure",
 *  "Work item"). A page route names none — the page is the thing, and its
 *  title says it. */
export function kindOfTab(uri: string | null, resources: ReadonlyMap<string, { label: string }>): string | null {
  const resource = uri ? typeOf(uri) : null;
  if (!resource || resource === "oxplow/page") return null;
  const label = resources.get(resource)?.label ?? "";
  return label === "" ? null : label;
}
