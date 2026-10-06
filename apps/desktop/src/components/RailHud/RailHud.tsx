import type { CSSProperties, ReactNode } from "react";
import { RAIL_SECTION_DRAG_MIME } from "../../dragMimes.js";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { PageKindIcon } from "../../pageKinds.js";
import type { TabRef } from "../../tabs/tabState.js";
import { refFromTabId, dashboardRef } from "../../tabs/pageRefs.js";
import { RAIL_HISTORY_EXCLUDE_KINDS } from "./history.js";
import { getPanelLayout, setPanelLayout } from "../../api.js";
import { useExtensions } from "../../extensionsStore.js";
import { NO_READS, useRerunOnChange } from "../../lens/lensRerun.js";
import { LensResultView } from "../../lens/LensResultView.js";
import type { ExtensionPanel, LensRun, PanelPlacement, Reads } from "../../tauri-bridge/generated/bindings.js";
import {
  CORE_PANELS,
  extensionPanelId,
  panelOpenRef,
  hidePanel,
  layoutSync,
  movePanelBeside,
  resolveLayout,
  revealPanel,
  setCollapsed,
  showPanel,
  type LayoutEdit,
  type LayoutSync,
} from "../Panels/panelLayout.js";
import type { PanelRuns } from "../Panels/usePanelRuns.js";
import { usePanelRuns } from "../Panels/PanelRunsContext.js";
import { useContextMenu } from "../useRowContextMenu.js";
import { recordOpError } from "../opErrorsStore.js";
import { readFailed } from "../../logger.js";
import {
  listRecentPageVisits,
  subscribePageVisitEvents,
  topVisitedPages,
  type PageVisitApi,
  type TopVisitedRowApi,
} from "../../api.js";
import { EmptyState } from "../Prompts/EmptyState.js";
import { readWikiPages } from "../../knowledge.js";

export interface BookmarkRailEntry {
  ref: TabRef;
  label: string;
}

export interface RailHudProps {
  threadId: string | null;
  /** Current stream — scopes the open-comments section. */
  streamId?: string | null;
  bookmarks?: BookmarkRailEntry[];
  /** Open a page (or focus if already open) in the active thread's tab area. */
  onOpenPage(ref: TabRef): void;
  /** Optional: invoked when the user clicks the search affordance. */
  onOpenSearch?(): void;
}

// ─── Uniform collapsible sections + drag-to-reorder ──────────────────
//
// Every content block in the rail renders through `RailSection`: a header
// with a drag handle, an expand/collapse chevron, the title, an optional
// count badge, and an optional header action. The person's layout —
// order, hidden, collapsed — persists per project in `panel_layout`
// (`get/set_panel_layout`). The Search box is
// pinned at the top and is not part of this set.

// A panel id: core's (`core:alerts`, …) or an extension's (`ext:<ext>/<id>`).
// "core:bookmarks" is the combined Bookmarks + History pane: collapsed it
// shows bookmarks only; expanded it adds the page-visit History list.
type RailSectionId = string;

interface RailSectionsValue {
  isExpanded(id: RailSectionId): boolean;
  setCollapsed(id: RailSectionId, collapsed: boolean): void;
  dragHandle(id: RailSectionId): {
    draggable: true;
    onDragStart(e: React.DragEvent): void;
    onDragEnd(): void;
  };
  dropZone(id: RailSectionId): {
    onDragOver(e: React.DragEvent): void;
    onDragLeave(): void;
    onDrop(e: React.DragEvent): void;
  };
  /** Which edge of `id` the insertion line should draw on (before/after),
   *  or null when this section isn't the current drop target. */
  dropSide(id: RailSectionId): "before" | "after" | null;
  /** Take the panel out of the nav (the Add Panel menu brings it back). */
  hide(id: RailSectionId): void;
}

const RailSectionsContext = createContext<RailSectionsValue | null>(null);

/** Owns the person's panel layout (`panel_layout`, through
 *  `get_panel_layout` / `set_panel_layout`: order, hidden, collapsed) and
 *  the in-flight drag state, for the panels `available` now. Exposes what
 *  `RailSection` needs through context. */
function useRailSections(available: string[]): {
  value: RailSectionsValue;
  order: RailSectionId[];
  hidden: RailSectionId[];
  /** The stored layout has arrived: what shows is the person's layout. */
  loaded: boolean;
  show(id: RailSectionId): void;
  reveal(id: RailSectionId): void;
} {
  const [stored, setStored] = useState<PanelPlacement[]>([]);
  const [draggingId, setDraggingId] = useState<RailSectionId | null>(null);
  const [dropTarget, setDropTarget] = useState<{ id: RailSectionId; side: "before" | "after" } | null>(null);
  // Kept in step with `panel_layout` by `layoutSync` (tsk972, tsk998): an
  // edit before the load is replayed on it, a failed load saves nothing,
  // saves land in order.
  const [layoutLoaded, setLayoutLoaded] = useState(false);
  const sync = useRef<LayoutSync | null>(null);
  useEffect(() => {
    const s = layoutSync({
      load: getPanelLayout,
      save: setPanelLayout,
      show: setStored,
      ready: () => setLayoutLoaded(true),
      failed: (label, e) => recordOpError({ label, message: e instanceof Error ? e.message : String(e) }),
    });
    sync.current = s;
    return () => {
      s.close();
      sync.current = null;
    };
  }, []);
  const layout = useMemo(() => resolveLayout(available, stored), [available, stored]);

  const edit = useCallback((change: LayoutEdit) => sync.current?.edit(change), []);

  const isExpanded = useCallback((id: RailSectionId) => !layout.collapsed.has(id), [layout]);
  const collapse = useCallback(
    (id: RailSectionId, collapsed: boolean) => edit((base) => setCollapsed(available, base, id, collapsed)),
    [available, edit],
  );
  const hide = useCallback((id: RailSectionId) => edit((base) => hidePanel(available, base, id)), [available, edit]);
  const show = useCallback((id: RailSectionId) => edit((base) => showPanel(available, base, id)), [available, edit]);
  const reveal = useCallback(
    (id: RailSectionId) => edit((base) => revealPanel(available, base, id)),
    [available, edit],
  );

  const dragHandle = useCallback((id: RailSectionId) => ({
    draggable: true as const,
    onDragStart(e: React.DragEvent) {
      e.dataTransfer.setData(RAIL_SECTION_DRAG_MIME, id);
      e.dataTransfer.effectAllowed = "move";
      setDraggingId(id);
    },
    onDragEnd() {
      setDraggingId(null);
      setDropTarget(null);
    },
  }), []);

  const dropZone = useCallback((id: RailSectionId) => ({
    onDragOver(e: React.DragEvent) {
      // Only react to our own section drag. Accept it everywhere (even
      // over the dragged section itself) so the insertion line tracks the
      // cursor and the cursor resolves to "move" (not the "+" copy icon).
      if (!draggingId) return;
      if (!e.dataTransfer.types.includes(RAIL_SECTION_DRAG_MIME)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      const rect = e.currentTarget.getBoundingClientRect();
      const side: "before" | "after" = e.clientY < rect.top + rect.height / 2 ? "before" : "after";
      if (dropTarget?.id !== id || dropTarget.side !== side) setDropTarget({ id, side });
    },
    onDragLeave() {
      if (dropTarget?.id === id) setDropTarget(null);
    },
    onDrop(e: React.DragEvent) {
      e.preventDefault();
      const sourceId = e.dataTransfer.getData(RAIL_SECTION_DRAG_MIME) || draggingId;
      setDraggingId(null);
      setDropTarget(null);
      if (!sourceId || sourceId === id) return;
      const rect = e.currentTarget.getBoundingClientRect();
      const side = e.clientY >= rect.top + rect.height / 2 ? "after" : "before";
      edit((base) => movePanelBeside(available, base, sourceId, id, side));
    },
  }), [draggingId, dropTarget, available, edit]);

  const dropSide = useCallback(
    (id: RailSectionId): "before" | "after" | null =>
      draggingId !== null && dropTarget?.id === id ? dropTarget.side : null,
    [dropTarget, draggingId],
  );

  const value = useMemo<RailSectionsValue>(
    () => ({ isExpanded, setCollapsed: collapse, dragHandle, dropZone, dropSide, hide }),
    [isExpanded, collapse, dragHandle, dropZone, dropSide, hide],
  );
  return { value, order: layout.order, hidden: layout.hidden, loaded: layoutLoaded, show, reveal };
}

/** Uniform section: drag handle + chevron + title (+ optional count and
 *  header action), then the collapsible body. When collapsed it renders
 *  `collapsedContent` (used by Work for its one-line summary) or nothing. */
function RailSection({
  id,
  title,
  count,
  tone,
  headerAction,
  collapsedContent,
  onOpen,
  openTitle,
  children,
}: {
  id: RailSectionId;
  title: string;
  count?: number;
  /** "danger" tints the title (Errors). */
  tone?: "danger";
  headerAction?: ReactNode;
  collapsedContent?: ReactNode;
  /** When set, a right-side icon opens this content's full page/dashboard. */
  onOpen?(): void;
  openTitle?: string;
  children: ReactNode;
}) {
  const ctx = useContext(RailSectionsContext);
  const expanded = ctx ? ctx.isExpanded(id) : true;
  const headerMenu = useContextMenu();
  const side = ctx ? ctx.dropSide(id) : null;
  const titleColor = tone === "danger" ? "var(--diff-del-fg, #f85149)" : "var(--text-secondary)";
  return (
    // Wrapper holds the inter-pane margin + drop-zone and (unlike the card)
    // is not overflow-clipped, so the insertion line can sit in the gap.
    <div
      data-testid={`rail-section-${id}`}
      {...(ctx ? ctx.dropZone(id) : {})}
      style={{
        position: "relative",
        margin: "0 6px 6px",
        // Don't let the flex column shrink panels when total height exceeds
        // the viewport — they stack and the column scrolls.
        flexShrink: 0,
      }}
    >
      {side ? (
        <span
          aria-hidden
          data-testid={`rail-section-drop-line-${id}-${side}`}
          style={{
            position: "absolute",
            left: 4,
            right: 4,
            [side === "before" ? "top" : "bottom"]: -4,
            height: 3,
            background: "var(--accent)",
            borderRadius: 2,
            pointerEvents: "none",
            zIndex: 2,
          }}
        />
      ) : null}
      <div
        style={{
          // Inset, rounded card (matching the Search box) that recesses
          // below the lighter rail; the gap between cards reveals the rail
          // (IntelliJ-style grouping).
          background: "var(--surface-card)",
          border: "1px solid var(--border-subtle)",
          borderRadius: 6,
          overflow: "hidden",
        }}
      >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 2,
          padding: "8px 8px 7px 6px",
          // Muted-accent header tint (not grey) so the title reads as an
          // intentional header band; the divider closes it off from the
          // content below.
          background: "var(--panel-header-bg)",
          borderBottom: expanded ? "1px solid var(--border-subtle)" : undefined,
        }}
        onContextMenu={(e) =>
          ctx
            ? headerMenu.open(e, [{ id: "hide-panel", label: "Hide Panel", enabled: true, run: () => ctx.hide(id) }])
            : undefined
        }
      >
        <span
          {...(ctx ? ctx.dragHandle(id) : {})}
          data-testid={`rail-section-drag-${id}`}
          title="Drag to reorder"
          aria-hidden
          style={{
            cursor: "grab",
            color: "var(--text-muted)",
            fontSize: 12,
            lineHeight: 1,
            padding: "0 2px",
            flexShrink: 0,
            userSelect: "none",
          }}
        >
          ⠿
        </span>
        <button
          type="button"
          data-testid={`rail-section-toggle-${id}`}
          onClick={() => ctx?.setCollapsed(id, expanded)}
          aria-expanded={expanded}
          title={expanded ? "Collapse" : "Expand"}
          style={{
            flex: 1,
            display: "flex",
            alignItems: "center",
            gap: 6,
            background: "transparent",
            border: "none",
            cursor: "pointer",
            padding: 0,
            textAlign: "left",
            fontSize: 11,
            fontWeight: 600,
            color: titleColor,
            textTransform: "uppercase",
            letterSpacing: 0.4,
          }}
        >
          <span aria-hidden style={{ width: 14, display: "inline-flex", justifyContent: "center", fontSize: 16, lineHeight: 1 }}>
            {expanded ? "▾" : "▸"}
          </span>
          <span>{title}</span>
          {count != null && count > 0 ? (
            <span style={{ color: "var(--text-muted)", fontSize: 11, fontWeight: 500 }}>{count}</span>
          ) : null}
        </button>
        {headerAction}
        {onOpen ? (
          <button
            type="button"
            data-testid={`rail-section-open-${id}`}
            onClick={(e) => { e.stopPropagation(); onOpen(); }}
            title={openTitle ?? "Open"}
            aria-label={openTitle ?? "Open"}
            style={{
              background: "transparent",
              border: "none",
              color: "var(--text-muted)",
              cursor: "pointer",
              fontSize: 13,
              lineHeight: 1,
              padding: "0 2px",
              flexShrink: 0,
            }}
          >
            ↗
          </button>
        ) : null}
      </div>
      {expanded ? children : (collapsedContent ?? null)}
      </div>
      {headerMenu.menu}
    </div>
  );
}

/**
 * Heads-up display rail. Always visible on the left; passive by design —
 * never auto-opens tabs. Sections only render when they have content.
 *
 * - Search button (opens the launcher — the single discovery surface)
 * - Active item summary
 * - Since you last looked  (TBD; placeholder for now)
 * - Ready
 * - Bookmarks (the user-curated pinned set; replaced the old Pages list)
 */
export function RailHud({
  threadId,
  streamId,
  bookmarks,
  onOpenPage,
  onOpenSearch,
}: RailHudProps) {
  const width = useRailWidth();
  // One owner for every panel's lens runs, shared with the bell and the
  // Alerts page (`PanelRunsProvider`, tsk1097).
  const { panels: extPanels, runs: panelRuns } = usePanelRuns();
  const available = useMemo(
    () => [...CORE_PANELS.map((p) => p.id), ...extPanels.map(extensionPanelId)],
    [extPanels],
  );
  const sections = useRailSections(available);
  const addMenu = useContextMenu();
  const titleOf = (id: string) =>
    CORE_PANELS.find((p) => p.id === id)?.title ?? extPanels.find((p) => extensionPanelId(p) === id)?.title ?? id;

  // Every visible panel always renders (a stable list); each shows its own
  // empty state when it has no content.
  function renderSection(id: RailSectionId): ReactNode {
    switch (id) {
      case "core:bookmarks":
        return <GoToSection key={id} entries={bookmarks ?? []} threadId={threadId} onOpenPage={onOpenPage} />;
      default: {
        const panel = extPanels.find((p) => extensionPanelId(p) === id);
        return panel ? (
          <ExtensionPanelSection
            key={id}
            panel={panel}
            runs={panelRuns[panel.id] ?? { body: null, badge: null, collapsed: null, count: null }}
            streamId={streamId ?? null}
            onOpenPage={onOpenPage}
          />
        ) : null;
      }
    }
  }

  return (
    <aside
      data-testid="rail-hud"
      // Busy until the person's stored layout has arrived: until then the
      // sections show their defaults.
      aria-busy={!sections.loaded}
      style={{
        width: width.value,
        flexShrink: 0,
        height: "100%",
        background: "var(--surface-chrome)",
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
        overflow: "hidden",
        position: "relative",
      }}
    >
      <div style={{ flex: 1, overflow: "auto", display: "flex", flexDirection: "column", minHeight: 0 }}>
        <SearchTrigger onOpenSearch={onOpenSearch} />
        <RailSectionsContext.Provider value={sections.value}>
          {sections.order.map((id) => renderSection(id))}
        </RailSectionsContext.Provider>
        {sections.hidden.length > 0 ? (
          <button
            type="button"
            data-testid="rail-add-panel"
            onClick={(e) =>
              addMenu.open(
                e,
                sections.hidden.map((id) => ({ id: `show-${id}`, label: titleOf(id), enabled: true, run: () => sections.show(id) })),
              )
            }
            style={{ margin: "0 6px 6px", background: "none", border: "1px dashed var(--border-subtle)", borderRadius: 6, padding: 4, color: "var(--text-muted)", cursor: "pointer", fontSize: 11 }}
          >
            + Add Panel
          </button>
        ) : null}
        {addMenu.menu}
      </div>
      <RailResizeHandle onChange={width.setFromDelta} />
    </aside>
  );
}

const RAIL_MIN_WIDTH = 260;
const RAIL_MAX_WIDTH = 600;
const RAIL_WIDTH_KEY = "oxplow.railWidth";

function useRailWidth() {
  const [value, setValue] = useState<number>(() => {
    if (typeof window === "undefined") return RAIL_MIN_WIDTH;
    const raw = window.localStorage.getItem(RAIL_WIDTH_KEY);
    const parsed = raw ? parseInt(raw, 10) : NaN;
    return Number.isFinite(parsed) ? clampRailWidth(parsed) : RAIL_MIN_WIDTH;
  });
  const startRef = useRef<{ start: number; base: number } | null>(null);
  useEffect(() => {
    if (typeof window === "undefined") return;
    window.localStorage.setItem(RAIL_WIDTH_KEY, String(value));
  }, [value]);
  const setFromDelta = useCallback((phase: "start" | "move" | "end", clientX: number) => {
    if (phase === "start") {
      startRef.current = { start: clientX, base: value };
      return;
    }
    if (!startRef.current) return;
    if (phase === "move") {
      const next = clampRailWidth(startRef.current.base + (clientX - startRef.current.start));
      setValue(next);
    } else if (phase === "end") {
      startRef.current = null;
    }
  }, [value]);
  return { value, setFromDelta };
}

function clampRailWidth(n: number) {
  return Math.max(RAIL_MIN_WIDTH, Math.min(RAIL_MAX_WIDTH, Math.round(n)));
}

function RailResizeHandle({ onChange }: { onChange(phase: "start" | "move" | "end", clientX: number): void }) {
  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    (e.currentTarget as HTMLDivElement).setPointerCapture(e.pointerId);
    onChange("start", e.clientX);
  };
  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.buttons === 0) return;
    onChange("move", e.clientX);
  };
  const onPointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    onChange("end", e.clientX);
    try { (e.currentTarget as HTMLDivElement).releasePointerCapture(e.pointerId); } catch {}
  };
  return (
    <div
      data-testid="rail-resize-handle"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      style={{
        position: "absolute",
        top: 0,
        right: 0,
        width: 6,
        height: "100%",
        cursor: "col-resize",
        userSelect: "none",
        zIndex: 5,
      }}
    />
  );
}

function SectionHeading({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        padding: "12px 14px 4px",
        fontSize: 11,
        fontWeight: 600,
        color: "var(--text-secondary)",
        textTransform: "uppercase",
        letterSpacing: 0.4,
      }}
    >
      {children}
    </div>
  );
}

const rowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "6px 14px",
  fontSize: "var(--text-sm)",
  color: "var(--text-primary)",
  cursor: "pointer",
  border: "none",
  background: "transparent",
  textAlign: "left",
  width: "100%",
  borderRadius: 0,
};

function rowHoverStyle(): CSSProperties {
  return { ...rowStyle };
}

function SearchTrigger({ onOpenSearch }: { onOpenSearch?: () => void }) {
  return (
    <div style={{ padding: "0 6px 6px", flexShrink: 0 }}>
      <button
        type="button"
        data-testid="rail-search"
        onClick={onOpenSearch}
        style={{
          width: "100%",
          padding: "8px 10px",
          background: "var(--surface-card)",
          border: "1px solid var(--border-subtle)",
          borderRadius: 6,
          color: "var(--text-secondary)",
          fontSize: "var(--text-sm)",
          textAlign: "left",
          cursor: onOpenSearch ? "pointer" : "default",
          display: "flex",
          alignItems: "center",
          gap: 8,
        }}
      >
        <span aria-hidden style={{ opacity: 0.7 }}>🔍</span>
        <span style={{ flex: 1 }}>Search…</span>
        <kbd
          style={{
            fontSize: 10,
            color: "var(--text-muted)",
            background: "var(--surface-tab-inactive)",
            padding: "1px 5px",
            borderRadius: 3,
            border: "1px solid var(--border-subtle)",
          }}
        >
          ⌘K
        </kbd>
      </button>
    </div>
  );
}

/** Muted empty-state line for a section that has no content but still
 *  renders (the pane list is stable — sections never disappear). */
function RailEmpty({ label }: { label: string }) {
  return (
    <div style={{ padding: "4px 14px 10px" }}>
      <EmptyState compact title={label} />
    </div>
  );
}

/** An extension's panel (P6.G1): its body lens, compact; its count in the
 *  header (`panelCount`: the count lens's, else the badge's while it
 *  fires); and, collapsed, its collapsed lens compact as the summary. */
function ExtensionPanelSection({
  panel,
  runs,
  streamId,
  onOpenPage,
}: {
  panel: ExtensionPanel;
  /** The panel's runs, from the rail's one owner (`useExtensionPanelRuns`). */
  runs: PanelRuns;
  streamId: string | null;
  onOpenPage(ref: TabRef): void;
}) {
  const compact = (run: LensRun, testId: string) => (
    <div data-testid={testId} style={{ padding: "4px 10px 8px", fontSize: "var(--text-xs)" }}>
      <LensResultView run={run} compact maxRows={8} streamId={streamId} onOpenPage={onOpenPage} />
    </div>
  );
  return (
    <RailSection
      id={extensionPanelId(panel)}
      title={panel.title}
      count={runs.count ?? undefined}
      collapsedContent={runs.collapsed ? compact(runs.collapsed, "rail-panel-collapsed") : undefined}
      onOpen={() => onOpenPage(panelOpenRef(panel))}
      openTitle={`Open ${panel.title}`}
    >
      {runs.body ? (
        compact(runs.body, "rail-panel-body")
      ) : (
        <div style={{ padding: "4px 10px 8px", fontSize: "var(--text-xs)" }}>
          <RailEmpty label="Loading…" />
        </div>
      )}
    </RailSection>
  );
}

/** The "Go To" pane — the rail's combined bookmarks + history surface.
 *  Collapsed it shows the bookmark rows only; expanded it labels them
 *  under a "Bookmarks" subheading and adds the page-visit History list
 *  (recent / most-visited, toggled inline). The ↗ opens the full
 *  "Go To" page where bookmarks are managed. */
function GoToSection({
  entries,
  threadId,
  onOpenPage,
}: {
  entries: BookmarkRailEntry[];
  threadId: string | null;
  onOpenPage(ref: TabRef): void;
}) {
  const history = useHistoryRows(threadId);

  const bookmarkRows = (
    <>
      {entries.length === 0 ? <RailEmpty label="No bookmarks" /> : null}
      <div data-testid="rail-bookmarks" style={{ paddingBottom: 8 }}>
        {entries.map((entry) => (
          <button
            key={entry.ref.id}
            type="button"
            data-testid={`rail-bookmark-${entry.ref.id}`}
            title={entry.label}
            onClick={() => onOpenPage(entry.ref)}
            style={rowHoverStyle()}
          >
            <PageKindIcon kind={entry.ref.kind} size={12} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {entry.label}
            </span>
          </button>
        ))}
      </div>
    </>
  );

  return (
    <RailSection
      id="core:bookmarks"
      title="Go To"
      onOpen={() => onOpenPage(dashboardRef("visits"))}
      openTitle="Open Go To"
      collapsedContent={bookmarkRows}
    >
      <SectionHeading>Bookmarks</SectionHeading>
      {bookmarkRows}
      <HistoryRows {...history} onOpenPage={onOpenPage} />
    </RailSection>
  );
}

interface HistoryRowsState {
  mode: "recent" | "top";
  toggleMode(): void;
  recent: PageVisitApi[];
  top: TopVisitedRowApi[];
  wikiTitles: Record<string, string>;
}

/** Page-visit history data for the combined Bookmarks pane: recent +
 *  most-visited rows, kept live, plus a fresh wiki slug→title map. */
function useHistoryRows(threadId: string | null): HistoryRowsState {
  const [mode, setMode] = useState<"recent" | "top">("recent");
  const [recent, setRecent] = useState<PageVisitApi[]>([]);
  const [top, setTop] = useState<TopVisitedRowApi[]>([]);
  // Wiki visit rows carry the title that was current when the page
  // was activated. That snapshot can be stale ("" for pages activated
  // before their summary loaded; outdated when titles change later).
  // Resolve fresh slug → title here and prefer it over `e.label`
  // whenever the entry is a wiki page.
  const [wikiTitles, setWikiTitles] = useState<Record<string, string>>({});

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void listRecentPageVisits({
        threadId,
        limit: 10,
        dedupeByRef: true,
        excludeKinds: RAIL_HISTORY_EXCLUDE_KINDS,
      }).then((rows) => {
        if (!cancelled) setRecent(rows);
      }, readFailed("recent pages"));
      const since = new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString();
      void topVisitedPages({
        threadId,
        sinceT: since,
        limit: 10,
        excludeKinds: RAIL_HISTORY_EXCLUDE_KINDS,
      }).then((rows) => {
        if (!cancelled) setTop(rows);
      }, readFailed("most visited pages"));
    };
    refresh();
    const off = subscribePageVisitEvents(refresh);
    return () => {
      cancelled = true;
      off();
    };
  }, [threadId]);

  // Maintain the slug → title map, re-read when a model it read changes
  // (a page created, renamed or deleted) so a renamed page updates in
  // the history list without waiting for the next visit.
  const [titleReads, setTitleReads] = useState<Reads>(NO_READS);
  const refreshTitles = useCallback(() => {
    let cancelled = false;
    void readWikiPages().then(({ pages, reads }) => {
      if (cancelled) return;
      const map: Record<string, string> = {};
      for (const p of pages) map[p.slug] = p.title;
      setWikiTitles(map);
      setTitleReads(reads);
    }, readFailed("wiki page titles"));
    return () => {
      cancelled = true;
    };
  }, []);
  useEffect(() => refreshTitles(), [refreshTitles]);
  useRerunOnChange(titleReads, () => void refreshTitles());

  const toggleMode = useCallback(() => setMode((m) => (m === "recent" ? "top" : "recent")), []);
  return { mode, toggleMode, recent, top, wikiTitles };
}

/** History block rendered inside the expanded Bookmarks pane: a
 *  "History" / "Most Visited" subheading with an inline recent/top
 *  toggle, then the visit rows. */
function HistoryRows({
  mode,
  toggleMode,
  recent,
  top,
  wikiTitles,
  onOpenPage,
}: HistoryRowsState & { onOpenPage(ref: TabRef): void }) {
  // If the user has data in only one of the two modes, fall back to
  // that one so the toggle doesn't render an empty list.
  const effectiveMode = mode === "recent" && recent.length === 0 && top.length > 0
    ? "top"
    : mode === "top" && top.length === 0 && recent.length > 0
    ? "recent"
    : mode;
  const source = effectiveMode === "recent" ? recent : top;
  const entries = source.slice(0, 10);
  const hasHistory = recent.length > 0 || top.length > 0;

  return (
    <>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          padding: "12px 14px 4px",
        }}
      >
        <span
          style={{
            flex: 1,
            fontSize: 11,
            fontWeight: 600,
            color: "var(--text-secondary)",
            textTransform: "uppercase",
            letterSpacing: 0.4,
          }}
        >
          {effectiveMode === "recent" ? "History" : "Most Visited"}
        </span>
        {hasHistory ? (
          <button
            type="button"
            data-testid="rail-history-mode"
            onClick={toggleMode}
            title={effectiveMode === "recent" ? "Show most visited (last 30d)" : "Show recent"}
            style={{
              background: "transparent",
              border: "none",
              color: "var(--text-secondary)",
              cursor: "pointer",
              fontSize: 10,
              padding: "0 4px",
            }}
          >
            {effectiveMode === "recent" ? "top" : "recent"}
          </button>
        ) : null}
      </div>
      {!hasHistory ? <RailEmpty label="No history yet" /> : null}
      <div data-testid="rail-history" style={{ paddingBottom: 4 }}>
        {entries.map((e) => {
          // Reconstruct the full ref (with payload) from the id —
          // page-visit rows don't persist payload, so a file ref needs
          // its `path` rebuilt or it won't open. See refFromTabId.
          const ref = refFromTabId(e.refId);
          if (!ref) return null;
          const trailing = effectiveMode === "top" ? (e as TopVisitedRowApi).count : null;
          // Wiki: prefer the live title over the stored visit label.
          // Falls back to a non-empty stored label, then to the slug,
          // so the row always renders something.
          const liveWikiTitle =
            ref.kind === "wiki" ? wikiTitles[e.refId]?.trim() : null;
          const display =
            liveWikiTitle || (e.label?.trim() ?? "") || e.refId;
          return (
            <button
              key={e.refId}
              type="button"
              data-testid={`rail-history-${e.refId}`}
              title={display}
              onClick={() => onOpenPage(ref)}
              style={rowHoverStyle()}
            >
              <PageKindIcon kind={ref.kind} size={12} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {display}
              </span>
              {trailing != null ? (
                <span
                  style={{
                    fontSize: 10,
                    color: "var(--text-secondary)",
                    background: "var(--surface-tab-inactive)",
                    padding: "1px 6px",
                    borderRadius: 999,
                  }}
                >
                  {trailing}
                </span>
              ) : null}
            </button>
          );
        })}
      </div>
    </>
  );
}
