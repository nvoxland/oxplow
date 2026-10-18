import type { CSSProperties, ReactNode } from "react";
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";

import {
  OPENED_SUFFIX,
  SECTIONS_COLLAPSED_KEY,
  allCollapsed,
  allExpanded,
  isExpanded as isExpandedIn,
  parseCollapsed,
  serializeCollapsed,
  setAll,
  toggleSection,
} from "./sectionCollapse.js";

/**
 * Collapsible page sections — a `<CollapsibleSections>` provider wrapping any
 * number of kit `<Section collapsible>`s (`kit/Section.tsx`, which reads the
 * provider through `useSectionCollapse`). Each section's heading carries an
 * expand/collapse toggle; `<SectionCollapseControls>` renders the "Expand all" /
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
 *   <Page width="reading" rightRail={<>…filters… <SectionCollapseControls /></>}>
 *     {groups.map((g) => (
 *       <Section key={g.key} id={g.key} title={g.label} collapsible>
 *         …
 *       </Section>
 *     ))}
 *   </Page>
 * </CollapsibleSections>
 * ```
 */

interface SectionsContextValue {
  isExpanded(id: string, defaultCollapsed: boolean): boolean;
  toggle(id: string, defaultCollapsed: boolean): void;
  /** Sections announce themselves on mount so the controls know what
   *  "all" means — only what's rendered right now counts. `startsClosed`:
   *  the section is `defaultCollapsed`. */
  register(id: string, startsClosed: boolean): void;
  unregister(id: string): void;
  /** The rendered section ids — what Expand/Collapse-all may act on. */
  rendered: readonly string[];
  /** Of those, the ones that start closed. */
  startClosed: ReadonlySet<string>;
  collapsed: ReadonlySet<string>;
  opened: ReadonlySet<string>;
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
  const [state, setState] = useState<{ collapsed: ReadonlySet<string>; opened: ReadonlySet<string> }>(() => ({
    collapsed: parseCollapsed(readStorage(), pageKey),
    opened: parseCollapsed(readStorage(), pageKey + OPENED_SUFFIX),
  }));
  const { collapsed, opened } = state;
  // The rendered section ids (and whether each starts closed) — only
  // membership matters (it decides whether Expand all / Collapse all have
  // anything left to do).
  const [ids, setIds] = useState<ReadonlyMap<string, boolean>>(() => new Map());

  const persist = useCallback(
    (next: { collapsed: ReadonlySet<string>; opened: ReadonlySet<string> }) => {
      setState(next);
      if (typeof window === "undefined") return;
      try {
        // Only exceptions are stored: collapsed ids of sections that start
        // open, opened ids of those that start closed.
        const withCollapsed = serializeCollapsed(readStorage(), pageKey, next.collapsed);
        window.localStorage.setItem(
          SECTIONS_COLLAPSED_KEY,
          serializeCollapsed(withCollapsed, pageKey + OPENED_SUFFIX, next.opened),
        );
      } catch {
        // Storage full / disabled — the toggle still works for this session.
      }
    },
    [pageKey],
  );

  const register = useCallback((id: string, startsClosed: boolean) => {
    setIds((prev) => (prev.get(id) === startsClosed ? prev : new Map(prev).set(id, startsClosed)));
  }, []);
  const unregister = useCallback((id: string) => {
    setIds((prev) => {
      if (!prev.has(id)) return prev;
      const next = new Map(prev);
      next.delete(id);
      return next;
    });
  }, []);

  const ctx = useMemo<SectionsContextValue>(
    () => ({
      isExpanded: (id, defaultCollapsed) => isExpandedIn(collapsed, id, opened, defaultCollapsed),
      toggle: (id, defaultCollapsed) => persist(toggleSection(state, id, defaultCollapsed)),
      register,
      unregister,
      rendered: [...ids.keys()],
      startClosed: new Set([...ids].filter(([, closed]) => closed).map(([id]) => id)),
      collapsed,
      opened,
      // Both act ONLY on what's on screen: a section remembered as collapsed but
      // currently filtered out keeps its state untouched.
      expandAll: () => persist(setAll(state, ids, true)),
      collapseAll: () => persist(setAll(state, ids, false)),
      testIdPrefix,
    }),
    [state, collapsed, opened, ids, persist, register, unregister, testIdPrefix],
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
  const { rendered, collapsed, opened, startClosed, testIdPrefix } = ctx;
  return (
    <div style={toolbarRow}>
      <button
        type="button"
        data-testid={`${testIdPrefix}-expand-all`}
        onClick={ctx.expandAll}
        disabled={allExpanded(rendered, collapsed, opened, startClosed)}
        title="Expand all sections"
        style={smallButton}
      >
        Expand all
      </button>
      <button
        type="button"
        data-testid={`${testIdPrefix}-collapse-all`}
        onClick={ctx.collapseAll}
        disabled={allCollapsed(rendered, collapsed, opened, startClosed)}
        title="Collapse all sections"
        style={smallButton}
      >
        Collapse all
      </button>
    </div>
  );
}

/**
 * What a collapsible kit `Section` reads: whether it is open, how to flip it,
 * and the testid prefix the page's provider chose. Registers the section so
 * Expand all / Collapse all know it is on screen. `null` outside a provider:
 * the section is then plain and always open.
 */
export function useSectionCollapse(
  id: string | undefined,
  enabled: boolean,
  defaultCollapsed: boolean,
): { expanded: boolean; toggle(): void; prefix: string } | null {
  const ctx = useContext(SectionsContext);
  const { register, unregister } = ctx ?? {};
  useEffect(() => {
    if (!enabled || !id || !register || !unregister) return;
    register(id, defaultCollapsed);
    return () => unregister(id);
  }, [enabled, id, defaultCollapsed, register, unregister]);
  if (!enabled || !id || !ctx) return null;
  return {
    expanded: ctx.isExpanded(id, defaultCollapsed),
    toggle: () => ctx.toggle(id, defaultCollapsed),
    prefix: ctx.testIdPrefix,
  };
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

