import type { ReactNode } from "react";
import { useState } from "react";
import { PageNavBar } from "./PageNavBar.js";
import { useOptionalPageNavigation } from "./PageNavigationContext.js";
import type { BookmarkScope } from "./bookmarks.js";

export interface PageNavBarConfig {
  canBack: boolean;
  canForward: boolean;
  onBack(): void;
  onForward(): void;
  bookmark?: {
    isBookmarked: boolean;
    onToggle(): void;
    scopes?: BookmarkScope[];
    defaultScope?: BookmarkScope;
    onToggleScope?(scope: BookmarkScope): void;
    onSetDefaultScope?(scope: BookmarkScope): void;
  };
  /** Backlinks dropdown content. Mutually exclusive with the
   *  legacy footer panel — when this is supplied, the footer is
   *  suppressed even if the `backlinks` prop is also set. */
  backlinks?: { count: number; body: ReactNode };
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
  /** Page title shown in the header. */
  title: ReactNode;
  /** Optional kind/type label rendered as a small chip ("file", "work item"…). */
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
  /** Optional nav-bar config. When supplied, the browser-style nav bar
   *  renders between header and body, and (if it carries a `backlinks`
   *  block) suppresses the legacy footer panel. */
  navBar?: PageNavBarConfig;
  /** Test id applied to the page root. */
  testId?: string;
}

/**
 * Shared chrome for every page rendered inside a tab body. Provides:
 *  - A header (title + kind chip + status/metadata chips + actions slot)
 *  - The page body
 *  - A collapsible Backlinks panel anchored at the bottom
 *
 * The chrome reads only semantic CSS variables. Both light and dark
 * themes are styled by `public/index.html`.
 */
export function Page({ title, kind, chips, actions, children, backlinks, navBar, testId }: PageProps) {
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
  const baseNavBar: PageNavBarConfig | undefined = navBar ?? (ctxNav ? {
    canBack: ctxNav.canGoBack,
    canForward: ctxNav.canGoForward,
    onBack: ctxNav.goBack,
    onForward: ctxNav.goForward,
    bookmark: ctxNav.bookmark
      ? {
          isBookmarked: ctxNav.bookmark.scopes.length > 0,
          onToggle: () => ctxNav.bookmark!.toggle(),
          scopes: ctxNav.bookmark.scopes,
          defaultScope: ctxNav.bookmark.lastScope,
          onToggleScope: (scope) => ctxNav.bookmark!.toggle(scope),
          onSetDefaultScope: (scope) => ctxNav.bookmark!.setLastScope(scope),
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
              fontSize: 17,
              fontWeight: 600,
              color: "var(--text-primary)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {title}
          </span>
          {kind ? (
            <span
              data-testid="page-kind"
              style={{
                fontSize: 11,
                fontWeight: 500,
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
                fontWeight: 500,
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
      {effectiveNavBar ? (
        <PageNavBar
          canBack={effectiveNavBar.canBack}
          canForward={effectiveNavBar.canForward}
          onBack={effectiveNavBar.onBack}
          onForward={effectiveNavBar.onForward}
          bookmark={effectiveNavBar.bookmark}
          backlinks={effectiveNavBar.backlinks}
          actions={effectiveNavBar.actions}
        />
      ) : null}
      <div style={{ flex: 1, minHeight: 0, minWidth: 0, overflow: "auto", display: "flex", flexDirection: "column" }}>
        {children}
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
              fontSize: 12,
              width: "100%",
              textAlign: "left",
            }}
            aria-expanded={backlinksOpen}
          >
            {backlinksOpen ? "▾" : "▸"} Backlinks
          </button>
          {backlinksOpen ? (
            <div data-testid="page-backlinks-body" style={{ padding: "0 16px 12px", fontSize: 12 }}>
              {backlinksBody}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
