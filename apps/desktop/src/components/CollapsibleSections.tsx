import type { CSSProperties, ReactNode } from "react";
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";

import {
  SECTIONS_COLLAPSED_KEY,
  allCollapsed,
  allExpanded,
  isExpanded as isExpandedIn,
  parseCollapsed,
  serializeCollapsed,
  toggleCollapsed,
} from "./sectionCollapse.js";

/**
 * Collapsible page sections (tsk84) — a `<CollapsibleSections>` provider wrapping
 * any number of `<CollapsibleSection>`s. Each section header carries an
 * expand/collapse chevron; `<SectionCollapseControls>` renders the "Expand all" /
 * "Collapse all" pair. The collapsed set persists per page.
 *
 * **Three parts, because the page owns placement (tsk86).** The provider holds
 * state only and renders `children` bare — no wrapper element — so it can wrap a
 * whole `<Page>` without disturbing the page chrome's `height: 100%` flex column.
 * That's what lets the controls live in the details **right rail** while the
 * sections live in the body: `rightRail` is *created* in the page but *rendered*
 * inside `Page`'s subtree, and React context follows the render tree, not the
 * creation site.
 *
 * **Still not a `Page` prop.** `Page` renders `children` opaquely and has no idea
 * what sections exist, so a page-layout flag couldn't draw the controls without
 * the sections registering themselves anyway — the flag would say only "the thing
 * I'm already doing is allowed". The page placing `<SectionCollapseControls />`
 * itself is both simpler and more flexible.
 *
 * Usage — controls in the details rail, sections in the body:
 * ```tsx
 * <CollapsibleSections pageKey="metrics-recorded" testIdPrefix="recorded">
 *   <Page layout="details" rightRail={<>…filters… <SectionCollapseControls /></>}>
 *     {groups.map((g) => (
 *       <CollapsibleSection key={g.key} id={g.key} title={g.label} count={g.entries.length}>
 *         …
 *       </CollapsibleSection>
 *     ))}
 *   </Page>
 * </CollapsibleSections>
 * ```
 */

interface SectionsContextValue {
  isExpanded(id: string): boolean;
  toggle(id: string): void;
  /** Sections announce themselves on mount so the controls know what
   *  "all" means — only what's rendered right now counts. */
  register(id: string): void;
  unregister(id: string): void;
  /** The rendered section ids — what Expand/Collapse-all may act on. */
  rendered: readonly string[];
  collapsed: ReadonlySet<string>;
  expandAll(): void;
  collapseAll(): void;
  testIdPrefix: string;
}

const SectionsContext = createContext<SectionsContextValue | null>(null);

function readStorage(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(SECTIONS_COLLAPSED_KEY);
  } catch {
    return null;
  }
}

export function CollapsibleSections({
  pageKey,
  testIdPrefix,
  children,
}: {
  /** Storage scope — one entry per page in the shared localStorage blob. */
  pageKey: string;
  /** Testid prefix: `<prefix>-expand-all`, `<prefix>-section-<id>`, … */
  testIdPrefix: string;
  children: ReactNode;
}) {
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() =>
    parseCollapsed(readStorage(), pageKey),
  );
  // The rendered section ids, in no particular order — only membership matters
  // (it decides whether Expand all / Collapse all have anything left to do).
  const [ids, setIds] = useState<ReadonlySet<string>>(() => new Set());

  const persist = useCallback(
    (next: ReadonlySet<string>) => {
      setCollapsed(next);
      if (typeof window === "undefined") return;
      try {
        window.localStorage.setItem(
          SECTIONS_COLLAPSED_KEY,
          serializeCollapsed(readStorage(), pageKey, next),
        );
      } catch {
        // Storage full / disabled — the toggle still works for this session.
      }
    },
    [pageKey],
  );

  const register = useCallback((id: string) => {
    setIds((prev) => (prev.has(id) ? prev : new Set(prev).add(id)));
  }, []);
  const unregister = useCallback((id: string) => {
    setIds((prev) => {
      if (!prev.has(id)) return prev;
      const next = new Set(prev);
      next.delete(id);
      return next;
    });
  }, []);

  const ctx = useMemo<SectionsContextValue>(
    () => ({
      isExpanded: (id) => isExpandedIn(collapsed, id),
      toggle: (id) => persist(toggleCollapsed(collapsed, id)),
      register,
      unregister,
      rendered: [...ids],
      collapsed,
      // Both act ONLY on what's on screen: a section remembered as collapsed but
      // currently filtered out keeps its state untouched.
      expandAll: () => persist(new Set([...collapsed].filter((id) => !ids.has(id)))),
      collapseAll: () => persist(new Set([...collapsed, ...ids])),
      testIdPrefix,
    }),
    [collapsed, ids, persist, register, unregister, testIdPrefix],
  );

  // Rendered bare — no wrapper element. This provider wraps a whole `<Page>` so
  // that context reaches both the details rail and the body, and any wrapper
  // here would sit between the tab body and the page chrome's `height: 100%`
  // column and break it.
  return <SectionsContext.Provider value={ctx}>{children}</SectionsContext.Provider>;
}

/**
 * The "Expand all" / "Collapse all" pair. Rendered wherever the page wants it —
 * on Metrics that's the details rail beside the filters (tsk86) — and
 * self-hides when no sections are registered (loading / empty state), rather
 * than showing two dead buttons.
 */
export function SectionCollapseControls() {
  const ctx = useContext(SectionsContext);
  if (!ctx || ctx.rendered.length === 0) return null;
  const { rendered, collapsed, testIdPrefix } = ctx;
  return (
    <div style={toolbarRow}>
      <button
        type="button"
        data-testid={`${testIdPrefix}-expand-all`}
        onClick={ctx.expandAll}
        disabled={allExpanded(rendered, collapsed)}
        title="Expand all sections"
        style={smallButton}
      >
        Expand all
      </button>
      <button
        type="button"
        data-testid={`${testIdPrefix}-collapse-all`}
        onClick={ctx.collapseAll}
        disabled={allCollapsed(rendered, collapsed)}
        title="Collapse all sections"
        style={smallButton}
      >
        Collapse all
      </button>
    </div>
  );
}

export function CollapsibleSection({
  id,
  title,
  count,
  children,
}: {
  id: string;
  title: ReactNode;
  /** Optional count badge — worth showing when the body is hidden. */
  count?: number;
  children: ReactNode;
}) {
  const ctx = useContext(SectionsContext);
  const { register, unregister } = ctx ?? {};
  useEffect(() => {
    if (!register || !unregister) return;
    register(id);
    return () => unregister(id);
  }, [id, register, unregister]);

  // Outside a provider the section still renders — just always open, so a
  // section component is never load-bearing on its context.
  const expanded = ctx ? ctx.isExpanded(id) : true;
  const prefix = ctx?.testIdPrefix ?? "section";

  return (
    <section data-testid={`${prefix}-group-${id}`}>
      {/* The toggle lives INSIDE an <h2> rather than replacing it: the section
          heading is still a heading for screen readers + document outline, and
          the button gets keyboard activation for free. */}
      <h2 style={headingWrap}>
        <button
          type="button"
          data-testid={`${prefix}-section-toggle-${id}`}
          onClick={() => ctx?.toggle(id)}
          aria-expanded={expanded}
          style={headerButton}
        >
          <span aria-hidden style={chevron}>
            {expanded ? "▾" : "▸"}
          </span>
          <span style={{ flex: 1, minWidth: 0, textAlign: "left" }}>{title}</span>
          {count != null ? <span style={countBadge}>{count}</span> : null}
        </button>
      </h2>
      {expanded ? <div data-testid={`${prefix}-section-body-${id}`}>{children}</div> : null}
    </section>
  );
}

const toolbarRow: CSSProperties = { display: "flex", gap: 6, alignItems: "center" };

const smallButton: CSSProperties = {
  padding: "4px 10px",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: "var(--text-xs)",
};

const headingWrap: CSSProperties = {
  margin: "0 0 8px",
  paddingBottom: 6,
  borderBottom: "1px solid var(--border-subtle)",
  fontSize: 17,
  fontWeight: 700,
};

const headerButton: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  width: "100%",
  border: "none",
  background: "transparent",
  color: "inherit",
  cursor: "pointer",
  padding: 0,
  font: "inherit",
};

const chevron: CSSProperties = {
  width: 14,
  flexShrink: 0,
  color: "var(--text-muted)",
  fontSize: 15,
  lineHeight: 1,
};

const countBadge: CSSProperties = {
  flexShrink: 0,
  fontSize: "var(--text-xs)",
  fontWeight: 500,
  color: "var(--text-muted)",
};
