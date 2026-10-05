import type { CSSProperties, ReactNode } from "react";
import { RAIL_SECTION_DRAG_MIME } from "../../dragMimes.js";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import type { DiffEntry, FinishedEntry, FileStatus, InProgressOp, ThreadWorkState, Task } from "../../api.js";
import { PageKindIcon } from "../../pageKinds.js";
import type { TabRef } from "../../tabs/tabState.js";
import { fileRef, wikiPageRef, tasksRef, uncommittedChangesRef, commentsRef, taskRef, refFromTabId, dashboardRef, lensRef, indexRef, workItemTabRef } from "../../tabs/pageRefs.js";
import { readWorkItems, type WorkItem } from "../../workItems.js";
import { setContextRefDrag } from "../../agent-context-dnd.js";
import { computeActiveEpicContext, computeActiveItem, computeUpNext } from "./sections.js";
import { RAIL_HISTORY_EXCLUDE_KINDS } from "./history.js";
import { getPanelLayout, setPanelLayout } from "../../api.js";
import { useExtensions } from "../../extensionsStore.js";
import { NO_READS, useRerunOnChange } from "../../lens/lensRerun.js";
import { LensResultView } from "../../lens/LensResultView.js";
import type { ExtensionPanel, PanelPlacement, Reads } from "../../tauri-bridge/generated/bindings.js";
import {
  CORE_PANELS,
  extensionPanelId,
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
import { decide, useProposals, type Proposal } from "../../proposals.js";
import { deliveryAlert, useUndelivered } from "../../delivery.js";
import { ProposalCard } from "../Proposals/ProposalCard.js";
import { panelAlerts, useExtensionPanelRuns, type PanelAlert, type PanelRuns } from "../Panels/usePanelRuns.js";
import { useContextMenu } from "../useRowContextMenu.js";
import { recordOpError } from "../opErrorsStore.js";
import { readFailed } from "../../logger.js";
import {
  listCommentsForStream,
  listRecentPageVisits,
  subscribeCommentEvents,
  subscribePageVisitEvents,
  topVisitedPages,
  type PageVisitApi,
  type TopVisitedRowApi,
} from "../../api.js";
import { EmptyState } from "../Prompts/EmptyState.js";
import { readWikiPages } from "../../knowledge.js";
import { goToSettingsSection } from "../../pages/settingsSections.js";

export interface UncommittedSummary {
  added: number;
  modified: number;
  deleted: number;
  additions: number;
  deletions: number;
  conflictedCount?: number;
  inProgress?: InProgressOp | null;
  /** The changed files — rendered as a tree when the section expands. */
  files?: DiffEntry[];
}

export interface BookmarkRailEntry {
  ref: TabRef;
  label: string;
}

export interface RailHudProps {
  threadId: string | null;
  /** Current stream — scopes the open-comments section. */
  streamId?: string | null;
  threadWork: ThreadWorkState | null;
  bookmarks?: BookmarkRailEntry[];
  /** Most recently finished work — closed tasks efforts merged
   *  with updated wiki notes, sorted by timestamp DESC. */
  recentlyFinished?: FinishedEntry[];
  /** Working-tree uncommitted summary; section hidden when null or empty. */
  uncommitted?: UncommittedSummary | null;
  /** Mark all currently-finished entries as seen (clears the section). */
  onClearFinished?(): void;
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

// A panel id: core's (`core:work`, …) or an extension's (`ext:<ext>/<id>`).
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
  threadWork,
  bookmarks,
  recentlyFinished,
  uncommitted,
  onClearFinished,
  onOpenPage,
  onOpenSearch,
}: RailHudProps) {
  const activeItem = useMemo(() => computeActiveItem(threadWork), [threadWork]);
  const activeEpic = useMemo(() => computeActiveEpicContext(threadWork, activeItem), [threadWork, activeItem]);
  // The full ready pool (capped) so the Work block can show an accurate
  // count even though it only renders the first handful when expanded.
  const readyItems = useMemo(() => computeUpNext(threadWork, 50), [threadWork]);
  const width = useRailWidth();
  const extPanels = useExtensionPanels(streamId ?? null);
  // One owner for every panel's lens runs: each section reads its own,
  // and Alerts is derived from the same runs.
  const panelRuns = useExtensionPanelRuns(extPanels, streamId ?? null, threadId);
  const alerts = useMemo(() => panelAlerts(extPanels, panelRuns), [extPanels, panelRuns]);
  const proposals = useProposals();
  const undelivered = useUndelivered();
  // Bumped by the Alerts row; Approvals scrolls itself into view.
  const [revealApprovals, setRevealApprovals] = useState(0);
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
      case "core:alerts":
        return (
          <AlertsSection
            key={id}
            alerts={alerts}
            proposals={proposals.length}
            undelivered={undelivered.length}
            onShowApprovals={() => {
              sections.reveal("core:approvals");
              setRevealApprovals((n) => n + 1);
            }}
            onOpenPage={onOpenPage}
          />
        );
      case "core:approvals":
        return <ApprovalsSection key={id} proposals={proposals} reveal={revealApprovals} />;
      case "core:uncommitted":
        return <UncommittedSection key={id} summary={uncommitted ?? null} onOpenPage={onOpenPage} />;
      case "core:comments":
        return <CommentsSection key={id} streamId={streamId ?? null} onOpenPage={onOpenPage} />;
      case "core:work":
        return (
          <WorkSection
            key={id}
            threadId={threadId}
            activeItem={activeItem}
            activeEpic={activeEpic}
            readyItems={readyItems}
            recentlyFinished={recentlyFinished}
            onOpenPage={onOpenPage}
            onClearFinished={onClearFinished}
          />
        );
      case "core:bookmarks":
        return <GoToSection key={id} entries={bookmarks ?? []} threadId={threadId} onOpenPage={onOpenPage} />;
      default: {
        const panel = extPanels.find((p) => extensionPanelId(p) === id);
        return panel ? (
          <ExtensionPanelSection
            key={id}
            panel={panel}
            runs={panelRuns[panel.id] ?? { body: null, badge: null, count: null }}
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

/** The Work section. Uniform collapsible header (id "work", default
 *  collapsed). When collapsed it keeps a compact one-liner — the active
 *  item when working, else the most recent finished item; expanding
 *  reveals the full In progress / Ready / Finished lists. */
function WorkSection({
  threadId,
  activeItem,
  activeEpic,
  readyItems,
  recentlyFinished,
  onOpenPage,
  onClearFinished,
}: {
  threadId: string | null;
  activeItem: Task | null;
  activeEpic: { epic: Task; children: Task[] } | null;
  readyItems: Task[];
  recentlyFinished?: FinishedEntry[];
  onOpenPage(ref: TabRef): void;
  onClearFinished?(): void;
}) {
  const finished = recentlyFinished ?? [];
  const readyCount = readyItems.length;
  const outside = useOutsideItems(threadId);
  const working = !!activeItem;
  const lastFinished = finished[0] ?? null;
  const hasContent = working || finished.length > 0 || readyCount > 0 || outside.length > 0;
  const isEmpty = !threadId || !hasContent;

  const collapsedContent = isEmpty ? (
    <RailEmpty label="No active work" />
  ) : working ? (
    <ActiveItemSection item={activeItem} epicContext={activeEpic} onOpenPage={onOpenPage} showHeading={false} />
  ) : lastFinished ? (
    <SingleFinishedRow entry={lastFinished} onOpenPage={onOpenPage} />
  ) : null;

  return (
    <RailSection
      id="core:work"
      title="Work"
      collapsedContent={collapsedContent}
      onOpen={() => onOpenPage(tasksRef())}
      openTitle="Open Tasks"
    >
      {isEmpty ? (
        <RailEmpty label="No active work" />
      ) : (
        <>
          {activeItem ? (
            <>
              <SectionHeading>In progress</SectionHeading>
              <ActiveItemSection item={activeItem} epicContext={activeEpic} onOpenPage={onOpenPage} showHeading={false} />
            </>
          ) : null}
          {readyCount > 0 ? (
            <UpNextSection items={readyItems.slice(0, 10)} onOpenPage={onOpenPage} />
          ) : null}
          {outside.length > 0 ? <OutsideSection items={outside.slice(0, 10)} onOpenPage={onOpenPage} /> : null}
          {finished.length > 0 ? (
            <FinishedSection entries={finished} onOpenPage={onOpenPage} onClear={onClearFinished} />
          ) : null}
        </>
      )}
    </RailSection>
  );
}

/** Single most-recent finished row — the collapsed "Last done" content
 *  when nothing is actively in progress. */
function SingleFinishedRow({
  entry,
  onOpenPage,
}: {
  entry: FinishedEntry;
  onOpenPage(ref: TabRef): void;
}) {
  const ref = entry.kind === "task" ? taskRef(entry.itemId) : wikiPageRef(entry.slug);
  return (
    <div data-testid="rail-last-done" style={{ paddingBottom: 8 }}>
      <button
        type="button"
        data-testid={`rail-finished-${entry.kind === "task" ? entry.itemId : entry.slug}`}
        title={entry.kind === "task" ? `#${entry.itemId} ${entry.title}` : entry.title}
        onClick={() => onOpenPage(ref)}
        style={rowHoverStyle()}
      >
        {entry.kind === "task" ? (
          <span
            aria-hidden
            style={{
              width: 14,
              display: "inline-flex",
              justifyContent: "center",
              color: statusIconColor("done"),
              fontSize: "var(--text-xs)",
              flexShrink: 0,
            }}
          >
            {statusIcon("done")}
          </span>
        ) : (
          <PageKindIcon kind="wiki" size={12} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
        )}
        <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {entry.title}
        </span>
      </button>
    </div>
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

function statusIcon(status: Task["status"]): string {
  switch (status) {
    case "done": return "✓";
    case "in_progress": return "◐";
    case "blocked": return "⚠";
    case "canceled": return "✗";
    case "archived": return "▣";
    case "ready":
    default: return "☐";
  }
}

function statusIconColor(status: Task["status"]): string {
  switch (status) {
    case "done": return "var(--diff-add-fg, #2ea043)";
    case "in_progress": return "var(--accent-fg, #58a6ff)";
    case "blocked": return "var(--diff-del-fg, #f85149)";
    case "canceled": return "var(--text-muted)";
    default: return "var(--text-secondary)";
  }
}

function ActiveItemSection({
  item,
  epicContext,
  onOpenPage,
  showHeading = true,
}: {
  item: Task | null;
  epicContext: { epic: Task; children: Task[] } | null;
  onOpenPage(ref: TabRef): void;
  /** Drop the "Current Work" heading when rendered inside the Work
   *  zone (the zone divider already labels it). */
  showHeading?: boolean;
}) {
  const [expanded, setExpanded] = useState(true);
  if (!item) {
    return null;
  }

  if (epicContext) {
    const { epic, children } = epicContext;
    return (
      <>
        {showHeading ? <SectionHeading>Current Work</SectionHeading> : null}
        <div
          data-testid="rail-active-epic"
          style={{
            display: "flex",
            alignItems: "center",
            gap: 4,
            padding: "4px 8px 4px 14px",
          }}
        >
          <button
            type="button"
            data-testid="rail-active-epic-toggle"
            onClick={() => setExpanded((v) => !v)}
            title={expanded ? "Collapse" : "Expand"}
            aria-expanded={expanded}
            style={{
              background: "transparent",
              border: "none",
              color: "var(--text-secondary)",
              cursor: "pointer",
              padding: "0 2px",
              fontSize: 14,
              width: 16,
            }}
          >
            {expanded ? "▾" : "▸"}
          </button>
          <button
            type="button"
            data-testid="rail-active-epic-row"
            title={`#${epic.id} ${epic.title}`}
            onClick={() => onOpenPage(taskRef(epic.id))}
            draggable
            onDragStart={(ev) => setContextRefDrag(ev, {
              kind: "task",
              itemId: epic.id,
              title: epic.title,
              status: epic.status,
            })}
            style={{
              ...rowStyle,
              padding: "2px 6px",
              flex: 1,
            }}
          >
            <span aria-hidden style={{ fontSize: 11, color: "var(--text-secondary)" }}>📚</span>
            <span
              style={{
                flex: 1,
                color: "var(--text-primary)",
                fontWeight: "var(--weight-medium)",
                fontSize: "var(--text-sm)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {epic.title}
            </span>
          </button>
        </div>
        {expanded ? (
          <div
            data-testid="rail-active-epic-children"
            style={{ paddingBottom: 8 }}
          >
            {children.map((child) => {
              const isActive = child.id === item.id;
              return (
                <button
                  key={child.id}
                  type="button"
                  data-testid={`rail-active-epic-child-${child.id}`}
                  onClick={() => onOpenPage(taskRef(child.id))}
                  draggable
                  onDragStart={(ev) => setContextRefDrag(ev, {
                    kind: "task",
                    itemId: child.id,
                    title: child.title,
                    status: child.status,
                  })}
                  style={{
                    ...rowStyle,
                    padding: "4px 14px 4px 32px",
                    background: isActive ? "var(--surface-card)" : "transparent",
                    fontWeight: isActive ? 500 : 400,
                  }}
                  title={`#${child.id} ${child.title}`}
                >
                  <span
                    aria-hidden
                    style={{
                      width: 14,
                      display: "inline-flex",
                      justifyContent: "center",
                      color: statusIconColor(child.status),
                      fontSize: "var(--text-xs)",
                    }}
                  >
                    {statusIcon(child.status)}
                  </span>
                  <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {child.title}
                  </span>
                </button>
              );
            })}
          </div>
        ) : null}
      </>
    );
  }

  return (
    <>
      {showHeading ? <SectionHeading>Current Work</SectionHeading> : null}
      <button
        type="button"
        data-testid="rail-active-item"
        title={`#${item.id} ${item.title}`}
        onClick={() => onOpenPage(taskRef(item.id))}
        draggable
        onDragStart={(ev) => setContextRefDrag(ev, {
          kind: "task",
          itemId: item.id,
          title: item.title,
          status: item.status,
        })}
        style={{
          ...rowStyle,
          padding: "4px 14px 12px",
        }}
      >
        <span
          aria-hidden
          style={{
            width: 14,
            display: "inline-flex",
            justifyContent: "center",
            color: statusIconColor(item.status),
            fontSize: "var(--text-xs)",
            flexShrink: 0,
          }}
        >
          {statusIcon(item.status)}
        </span>
        <span
          style={{
            flex: 1,
            color: "var(--text-primary)",
            fontWeight: "var(--weight-medium)",
            fontSize: "var(--text-sm)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {item.title}
        </span>
      </button>
    </>
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

type UStatus = "added" | "modified" | "deleted";
const U_STATUS_META: Record<UStatus, { letter: string; color: string }> = {
  added: { letter: "A", color: "var(--diff-add-fg, #2ea043)" },
  modified: { letter: "M", color: "var(--status-waiting, #f59e0b)" },
  deleted: { letter: "D", color: "var(--diff-del-fg, #f85149)" },
};
function uStatus(s: FileStatus): UStatus {
  if (s === "deleted") return "deleted";
  if (s === "added" || s === "untracked") return "added";
  return "modified";
}

interface UDirNode { type: "dir"; name: string; path: string; statuses: Set<UStatus>; children: UNode[]; }
interface UFileNode { type: "file"; name: string; path: string; status: UStatus; }
type UNode = UDirNode | UFileNode;

/** Folder>file tree, with each folder carrying the union of A/M/D
 *  statuses across its subtree. */
function buildUncommittedTree(files: DiffEntry[]): UNode[] {
  interface Raw { name: string; path: string; files: DiffEntry[]; dirs: Map<string, Raw>; }
  const root: Raw = { name: "", path: "", files: [], dirs: new Map() };
  for (const f of files) {
    const segs = f.path.split("/");
    let cur = root;
    for (let i = 0; i < segs.length - 1; i++) {
      const seg = segs[i]!;
      let next = cur.dirs.get(seg);
      if (!next) { const p = cur.path ? `${cur.path}/${seg}` : seg; next = { name: seg, path: p, files: [], dirs: new Map() }; cur.dirs.set(seg, next); }
      cur = next;
    }
    cur.files.push(f);
  }
  const materialize = (node: Raw): { nodes: UNode[]; statuses: Set<UStatus> } => {
    const out: UNode[] = [];
    const agg = new Set<UStatus>();
    for (const d of [...node.dirs.values()].sort((a, b) => a.name.localeCompare(b.name))) {
      const sub = materialize(d);
      for (const s of sub.statuses) agg.add(s);
      out.push({ type: "dir", name: d.name, path: d.path, statuses: sub.statuses, children: sub.nodes });
    }
    for (const f of [...node.files].sort((a, b) => a.path.localeCompare(b.path))) {
      const st = uStatus(f.status);
      agg.add(st);
      out.push({ type: "file", name: f.path.split("/").pop() ?? f.path, path: f.path, status: st });
    }
    return { nodes: out, statuses: agg };
  };
  return materialize(root).nodes;
}

const UStatusLetters = ({ statuses }: { statuses: Set<UStatus> }) => {
  const order: UStatus[] = ["added", "modified", "deleted"];
  return (
    <span style={{ display: "inline-flex", gap: 3, flexShrink: 0 }}>
      {order.filter((s) => statuses.has(s)).map((s) => (
        <span key={s} style={{ color: U_STATUS_META[s].color, fontSize: 10, fontWeight: 700 }}>{U_STATUS_META[s].letter}</span>
      ))}
    </span>
  );
};

const railTreeRowStyle: CSSProperties = {
  display: "flex", alignItems: "center", gap: 6, width: "100%",
  padding: "3px 8px", border: "none", background: "transparent",
  color: "var(--text-primary)", cursor: "pointer", textAlign: "left",
  fontSize: "var(--text-xs)",
};
const railTreeLabelStyle: CSSProperties = {
  flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
};

/** Compact uncommitted file tree for the rail — folders show their A/M/D
 *  union, files their status, with a floating expand/collapse-all toggle
 *  and a capped, scrollable height. */
function UncommittedTree({ files, onOpenFile }: { files: DiffEntry[]; onOpenFile(path: string): void }) {
  const tree = useMemo(() => buildUncommittedTree(files), [files]);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const toggle = (path: string) => setCollapsed((prev) => {
    const n = new Set(prev);
    if (n.has(path)) n.delete(path); else n.add(path);
    return n;
  });

  const rows: ReactNode[] = [];
  const walk = (nodes: UNode[], depth: number) => {
    for (const node of nodes) {
      const pad = 8 + depth * 12;
      if (node.type === "dir") {
        const isCollapsed = collapsed.has(node.path);
        rows.push(
          <button key={`d:${node.path}`} type="button" onClick={() => toggle(node.path)} title={node.path} style={{ ...railTreeRowStyle, paddingLeft: pad }}>
            <span aria-hidden style={{ width: 14, flexShrink: 0, color: "var(--text-muted)", fontSize: 15, lineHeight: 1 }}>{isCollapsed ? "▸" : "▾"}</span>
            <span style={railTreeLabelStyle}>{node.name}/</span>
            <UStatusLetters statuses={node.statuses} />
          </button>,
        );
        if (!isCollapsed) walk(node.children, depth + 1);
      } else {
        rows.push(
          <button key={`f:${node.path}`} type="button" data-testid={`rail-uncommitted-file-${node.path}`} onClick={() => onOpenFile(node.path)} title={node.path} style={{ ...railTreeRowStyle, paddingLeft: pad + 12 }}>
            <span style={railTreeLabelStyle}>{node.name}</span>
            <span style={{ color: U_STATUS_META[node.status].color, fontSize: 10, fontWeight: 700, flexShrink: 0 }}>{U_STATUS_META[node.status].letter}</span>
          </button>,
        );
      }
    }
  };
  walk(tree, 0);

  return (
    <div data-testid="rail-uncommitted-tree" style={{ maxHeight: 300, overflowY: "auto", overflowX: "hidden", paddingBottom: 6 }}>
      {rows}
    </div>
  );
}

function UncommittedSection({
  summary,
  onOpenPage,
}: {
  summary: UncommittedSummary | null;
  onOpenPage(ref: TabRef): void;
}) {
  const added = summary?.added ?? 0;
  const modified = summary?.modified ?? 0;
  const deleted = summary?.deleted ?? 0;
  const segs: { label: string; color: string }[] = [];
  if (added > 0) segs.push({ label: `${added}A`, color: U_STATUS_META.added.color });
  if (modified > 0) segs.push({ label: `${modified}M`, color: U_STATUS_META.modified.color });
  if (deleted > 0) segs.push({ label: `${deleted}D`, color: U_STATUS_META.deleted.color });
  const conflictedCount = summary?.conflictedCount ?? 0;
  const op = summary?.inProgress ? summary.inProgress.replace("_", "-") : null;
  const hasFileSummary = segs.length > 0;
  const hasConflictRow = conflictedCount > 0 || op !== null;
  const files = summary?.files ?? [];
  const total = added + modified + deleted;

  const summaryButton = hasFileSummary ? (
    <button
      type="button"
      data-testid="rail-uncommitted"
      onClick={() => onOpenPage(uncommittedChangesRef())}
      title="Open uncommitted changes"
      style={{ ...rowStyle, padding: "4px 14px 8px", gap: 8 }}
    >
      <span style={{ fontSize: "var(--text-xs)", display: "inline-flex", gap: 6, fontWeight: 600 }}>
        {segs.map((s) => (
          <span key={s.label} style={{ color: s.color }}>{s.label}</span>
        ))}
      </span>
      <span style={{ flex: 1 }} />
      <span style={{ color: "var(--diff-add-fg, #2ea043)", fontSize: 11 }}>+{summary?.additions ?? 0}</span>
      <span style={{ color: "var(--diff-del-fg, #f85149)", fontSize: 11 }}>−{summary?.deletions ?? 0}</span>
    </button>
  ) : null;

  const conflictRow = hasConflictRow ? (
    <button
      type="button"
      data-testid="rail-uncommitted-conflicts"
      onClick={() => onOpenPage(uncommittedChangesRef())}
      title={
        op
          ? `${op} in progress${conflictedCount > 0 ? ` — ${conflictedCount} conflicted file${conflictedCount === 1 ? "" : "s"}` : ""}`
          : `${conflictedCount} conflicted file${conflictedCount === 1 ? "" : "s"}`
      }
      style={{ ...rowStyle, padding: "4px 14px 8px", gap: 8 }}
    >
      <span style={{ color: "var(--diff-del-fg, #f85149)", fontSize: "var(--text-xs)" }}>
        {op ? `${op} in progress` : `${conflictedCount} conflict${conflictedCount === 1 ? "" : "s"}`}
      </span>
    </button>
  ) : null;

  const isEmpty = !hasFileSummary && !hasConflictRow;

  return (
    <RailSection
      id="core:uncommitted"
      title="Uncommitted"
      count={total}
      onOpen={() => onOpenPage(uncommittedChangesRef())}
      openTitle="Open uncommitted changes"
      collapsedContent={isEmpty ? <RailEmpty label="Working tree clean" /> : <>{summaryButton}{conflictRow}</>}
    >
      {conflictRow}
      {files.length > 0 ? (
        <UncommittedTree files={files} onOpenFile={(path) => onOpenPage(fileRef(path))} />
      ) : isEmpty ? (
        <RailEmpty label="Working tree clean" />
      ) : null}
    </RailSection>
  );
}

/// Open-comments summary: counts of unresolved comments in the current
/// stream, split by intent — "for me" (notes-to-self) and "for the
/// agent" (follow-ups). Self-fetching + live like the history rows;
/// hidden when there are none. Each row opens the Comments inbox.
/** Alerts (a core panel, P6.G1): the proposals waiting for the person
 *  (one leading row that reveals Approvals), then every panel badge that
 *  fires, one row each with its message, opening the badge's lens; and
 *  one row while events couldn't be delivered, opening Settings (Data →
 *  Delivery). Live. */
function AlertsSection({
  alerts,
  proposals,
  undelivered,
  onShowApprovals,
  onOpenPage,
}: {
  alerts: PanelAlert[];
  /** How many proposals are pending. */
  proposals: number;
  /** How many events wait in the dead-letter queue. */
  undelivered: number;
  onShowApprovals(): void;
  onOpenPage(ref: TabRef): void;
}) {
  const delivery = deliveryAlert(undelivered);
  const count = alerts.length + (proposals > 0 ? 1 : 0) + (delivery ? 1 : 0);
  return (
    <RailSection id="core:alerts" title="Alerts" count={count || undefined}>
      {count === 0 ? <RailEmpty label="Nothing needs you" /> : null}
      {proposals > 0 ? (
        <button
          type="button"
          data-testid="rail-alert-proposals"
          onClick={onShowApprovals}
          title="Show Approvals"
          style={{ ...rowStyle, padding: "4px 14px 4px", gap: 8 }}
        >
          <span style={{ color: "var(--accent)", fontSize: "var(--text-xs)" }}>
            {proposals === 1 ? "1 proposal awaits your approval" : `${proposals} proposals await your approval`}
          </span>
        </button>
      ) : null}
      {delivery ? (
        <button
          type="button"
          data-testid="rail-alert-delivery"
          onClick={() => {
            goToSettingsSection("settings-data-delivery");
            onOpenPage(indexRef("settings"));
          }}
          title="Open Settings → Data → Delivery"
          style={{ ...rowStyle, padding: "4px 14px 4px", gap: 8 }}
        >
          <span style={{ color: "var(--severity-critical)", fontSize: "var(--text-xs)" }}>{delivery}</span>
        </button>
      ) : null}
      {alerts.map((a) => (
        <button
          key={a.id}
          type="button"
          data-testid={`rail-alert-${a.id}`}
          onClick={() => onOpenPage(lensRef(a.id))}
          title={`Open ${a.title}`}
          style={{ ...rowStyle, padding: "4px 14px 4px", gap: 8 }}
        >
          <span style={{ color: "var(--text-primary)", fontSize: "var(--text-xs)" }}>{a.title}</span>
          <span style={{ flex: 1 }} />
          <span style={{ color: "var(--accent)", fontSize: 11 }}>{a.message}</span>
        </button>
      ))}
    </RailSection>
  );
}

/** Approvals (a core panel, P6b.A4): the agent's runs that wait for the
 *  person, newest first, each with Approve and Decline. */
function ApprovalsSection({ proposals, reveal }: { proposals: Proposal[]; reveal: number }) {
  // Asked to show itself (the Alerts row): once mounted with the new
  // layout, scroll it into view.
  const ref = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (reveal > 0) ref.current?.scrollIntoView?.({ block: "nearest" });
  }, [reveal]);
  return (
    <div ref={ref}>
      <RailSection id="core:approvals" title="Approvals" count={proposals.length || undefined}>
        {proposals.length === 0 ? <RailEmpty label="Nothing waits for your approval" /> : null}
        <div style={{ display: "flex", flexDirection: "column", gap: 6, padding: proposals.length ? "6px 8px 8px" : 0 }}>
          {proposals.map((p) => (
            <ProposalCard key={p.id} proposal={p} onDecide={decide} />
          ))}
        </div>
      </RailSection>
    </div>
  );
}

/** An extension's panel (P6.G1): its body lens, compact, and its badge's
 *  count in the header while the badge fires. */
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
  return (
    <RailSection
      id={extensionPanelId(panel)}
      title={panel.title}
      count={runs.count ?? undefined}
      onOpen={() => onOpenPage(lensRef(panel.body))}
      openTitle={`Open ${panel.title}`}
    >
      <div style={{ padding: "4px 10px 8px", fontSize: "var(--text-xs)" }}>
        {runs.body ? (
          <LensResultView run={runs.body} compact maxRows={8} streamId={streamId} onOpenPage={onOpenPage} />
        ) : (
          <RailEmpty label="Loading…" />
        )}
      </div>
    </RailSection>
  );
}

/** The enabled extensions' panels for this stream (the shared extensions
 *  store, so it follows their changes). */
function useExtensionPanels(streamId: string | null): ExtensionPanel[] {
  const exts = useExtensions(streamId);
  return useMemo(() => (exts ?? []).filter((e) => e.enabled).flatMap((e) => e.panels), [exts]);
}

function CommentsSection({
  streamId,
  onOpenPage,
}: {
  streamId: string | null;
  onOpenPage(ref: TabRef): void;
}) {
  const [notes, setNotes] = useState(0);
  const [followups, setFollowups] = useState(0);

  useEffect(() => {
    if (!streamId) {
      setNotes(0);
      setFollowups(0);
      return;
    }
    let cancelled = false;
    const refresh = () => {
      void listCommentsForStream(streamId).then((threads) => {
        if (cancelled) return;
        let n = 0;
        let f = 0;
        for (const t of threads) {
          if (t.comment.status !== "open") continue;
          if (t.comment.intent === "followup") f += 1;
          else n += 1;
        }
        setNotes(n);
        setFollowups(f);
      });
    };
    refresh();
    const off = subscribeCommentEvents(refresh);
    return () => {
      cancelled = true;
      off();
    };
  }, [streamId]);

  const isEmpty = notes === 0 && followups === 0;

  return (
    <RailSection
      id="core:comments"
      title="Comments"
      onOpen={() => onOpenPage(commentsRef())}
      openTitle="Open the Comments inbox"
    >
      {isEmpty ? <RailEmpty label="No open comments" /> : null}
      {notes > 0 ? (
        <button
          type="button"
          data-testid="rail-comments-notes"
          onClick={() => onOpenPage(commentsRef())}
          title="Open the Comments inbox"
          style={{ ...rowStyle, padding: "4px 14px 4px", gap: 8 }}
        >
          <span style={{ color: "var(--text-primary)", fontSize: "var(--text-xs)" }}>
            For me
          </span>
          <span style={{ flex: 1 }} />
          <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>{notes}</span>
        </button>
      ) : null}
      {followups > 0 ? (
        <button
          type="button"
          data-testid="rail-comments-followups"
          onClick={() => onOpenPage(commentsRef())}
          title="Open the Comments inbox"
          style={{ ...rowStyle, padding: "4px 14px 12px", gap: 8 }}
        >
          <span style={{ color: "var(--text-primary)", fontSize: "var(--text-xs)" }}>
            For the agent
          </span>
          <span style={{ flex: 1 }} />
          <span style={{ color: "var(--accent)", fontSize: 11 }}>{followups}</span>
        </button>
      ) : null}
    </RailSection>
  );
}

/** The thread's open items on another provider — an outside tracker the
 *  agent filed them on (tsk1041); oxplow's own tasks are `threadWork`. */
function useOutsideItems(threadId: string | null): WorkItem[] {
  const [items, setItems] = useState<WorkItem[]>([]);
  const [reads, setReads] = useState(NO_READS);
  const load = useCallback(async () => {
    if (!threadId) {
      setItems([]);
      return;
    }
    try {
      const read = await readWorkItems({ scope: { thread: threadId }, states: ["todo", "in_progress", "blocked"] });
      setItems(read.items.filter((i) => i.provider !== "oxplow"));
      setReads(read.reads);
    } catch {
      setItems([]);
    }
  }, [threadId]);
  useEffect(() => {
    void load();
  }, [load]);
  useRerunOnChange(reads, () => void load());
  return items;
}

/** The thread's open items on an outside tracker, each opening its page. */
function OutsideSection({ items, onOpenPage }: { items: WorkItem[]; onOpenPage(ref: TabRef): void }) {
  return (
    <>
      <SectionHeading>On your tracker</SectionHeading>
      <div data-testid="rail-outside-items" style={{ paddingBottom: 8 }}>
        {items.map((item) => (
          <button
            key={item.ref}
            type="button"
            data-testid={`rail-outside-item-${item.ref}`}
            title={`${item.title} (${item.provider}, ${item.state.replace("_", " ")})`}
            onClick={() => onOpenPage(workItemTabRef(item.ref))}
            style={rowHoverStyle()}
          >
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{item.title}</span>
            <span style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>{item.provider}</span>
          </button>
        ))}
      </div>
    </>
  );
}

function UpNextSection({
  items,
  onOpenPage,
}: {
  items: Task[];
  onOpenPage(ref: TabRef): void;
}) {
  return (
    <>
      <SectionHeading>Ready</SectionHeading>
      <div data-testid="rail-up-next" style={{ paddingBottom: 8 }}>
        {items.map((item) => (
          <button
            key={item.id}
            type="button"
            data-testid={`rail-up-next-item-${item.id}`}
            title={`#${item.id} ${item.title}`}
            onClick={() => onOpenPage(taskRef(item.id))}
            draggable
            onDragStart={(ev) => setContextRefDrag(ev, {
              kind: "task",
              itemId: item.id,
              title: item.title,
              status: item.status,
            })}
            style={rowHoverStyle()}
          >
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {item.title}
            </span>
          </button>
        ))}
      </div>
    </>
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

function FinishedSection({
  entries,
  onOpenPage,
  onClear,
}: {
  entries: FinishedEntry[];
  onOpenPage(ref: TabRef): void;
  onClear?(): void;
}) {
  return (
    <>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          padding: "12px 14px 4px",
        }}
      >
        <button
          type="button"
          data-testid="rail-finished-heading"
          onClick={() => onOpenPage(tasksRef())}
          title="Open Tasks"
          style={{
            flex: 1,
            background: "transparent",
            border: "none",
            padding: 0,
            margin: 0,
            textAlign: "left",
            cursor: "pointer",
            fontSize: 11,
            fontWeight: 600,
            color: "var(--text-secondary)",
            textTransform: "uppercase",
            letterSpacing: 0.4,
          }}
        >
          Finished
        </button>
        {onClear ? (
          <button
            type="button"
            data-testid="rail-finished-clear"
            onClick={(e) => { e.stopPropagation(); onClear(); }}
            title="Mark all as seen"
            style={{
              background: "transparent",
              border: "none",
              color: "var(--text-secondary)",
              cursor: "pointer",
              fontSize: 10,
              padding: "0 4px",
            }}
          >
            clear
          </button>
        ) : null}
      </div>
      <div data-testid="rail-finished" style={{ paddingBottom: 8 }}>
        {entries.map((e) => {
          const ref = e.kind === "task" ? taskRef(e.itemId) : wikiPageRef(e.slug);
          return (
            <button
              key={`${e.kind}:${e.kind === "task" ? e.itemId : e.slug}`}
              type="button"
              data-testid={`rail-finished-${e.kind === "task" ? e.itemId : e.slug}`}
              title={e.kind === "task" ? `#${e.itemId} ${e.title}` : e.title}
              onClick={() => onOpenPage(ref)}
              style={rowHoverStyle()}
            >
              <PageKindIcon kind={e.kind === "task" ? "work_item" : "wiki"} size={12} style={{ color: "var(--text-secondary)", flexShrink: 0 }} />
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {e.title}
              </span>
            </button>
          );
        })}
      </div>
    </>
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
