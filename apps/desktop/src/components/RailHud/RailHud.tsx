import { iconFor } from "../../icons.js";
import type { ReactNode } from "react";
import { Fragment } from "react";
import { useAppPreference } from "../../prefs/remembered.js";
import { RAIL_SECTION_DRAG_MIME } from "../../dragMimes.js";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { Layers, MessageSquare, Minimize2 } from "lucide-react";
import type { TabRef } from "../../tabs/tabState.js";
import { useExtensions } from "../../extensionsStore.js";
import type { ExtensionPanel } from "../../tauri-bridge/generated/bindings.js";
import { extensionPanelId, panelOpenRef } from "../Panels/panelLayout.js";
import { usePanelLayout } from "../Panels/PanelLayoutContext.js";
import type { PanelGroup } from "../Panels/panelLayout.js";
import { visibilityScopeLabel } from "../../visibilityScope.js";
import { railPanelItems } from "../../menuBar.js";
import { useDragAutoScroll } from "./dragAutoScroll.js";
import { type PanelChoice, type PanelRuns } from "../Panels/usePanelRuns.js";
import { PageTree } from "../../pages/PageTree.js";
import { usePanelRuns } from "../Panels/PanelRunsContext.js";
import { useContextMenu } from "../useRowContextMenu.js";
import { EmptyState } from "../Prompts/EmptyState.js";
import { OpenPageLink } from "../../kit/OpenPageLink.js";

export interface RailHudProps {
  /** Current stream — what a panel's page actions run in. */
  streamId?: string | null;
  /** Open a page (or focus if already open) in the active thread's tab area. */
  onOpenPage(ref: TabRef): void;
}

// ─── Uniform collapsible sections + drag-to-reorder ──────────────────
//
// Every content block in the rail renders through `RailSection`: a header
// with a drag handle, an expand/collapse chevron, the title, an optional
// count badge, and an optional header action. The person's layout —
// order, hidden, collapsed — persists per project in `panel_layout`
// (`get/set_panel_layout`). The Search box is
// pinned at the top and is not part of this set.

// A panel id: an extension's (`ext:<ext>/<id>`) — core has no panels of
// its own.
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
    onDragEnter(e: React.DragEvent): void;
    onDragOver(e: React.DragEvent): void;
    onDragLeave(): void;
    onDrop(e: React.DragEvent): void;
  };
  /** Which edge of `id` the insertion line should draw on (before/after),
   *  or null when this section isn't the current drop target. */
  dropSide(id: RailSectionId): "before" | "after" | null;
  /** Take the panel out of the nav (View ▸ Sidebar Panels brings it back). */
  hide(id: RailSectionId): void;
  /** The group a shown panel is in: this thread's, this stream's, the project's. */
  groupOf(id: RailSectionId): PanelGroup | undefined;
  /** The groups the viewer has. */
  groups: PanelGroup[];
  /** Move the panel to the end of `group` (Show in). */
  moveToGroup(id: RailSectionId, group: PanelGroup): void;
}

const RailSectionsContext = createContext<RailSectionsValue | null>(null);

/** The in-flight drag state over the viewer's layout (`usePanelLayout`,
 *  the person's order, groups and collapsed panels). Exposes what
 *  `RailSection` needs through context. */
function useRailSections(): {
  value: RailSectionsValue;
  /** Shown panels per group, top to bottom. */
  groups: Record<PanelGroup, RailSectionId[]>;
  /** The viewer's groups, top to bottom. */
  groupList: PanelGroup[];
  /** A panel is being dragged: the groups' dividers and the Remove zone show. */
  dragging: boolean;
  /** A group's divider, or the Remove zone (`null`), as a drop target. */
  groupDrop(group: PanelGroup | null): {
    onDragEnter(e: React.DragEvent): void;
    onDragOver(e: React.DragEvent): void;
    onDragLeave(): void;
    onDrop(e: React.DragEvent): void;
  };
  /** The divider or zone a drag is over now (`"remove"` for the zone). */
  groupDropTarget: PanelGroup | "remove" | null;
  order: RailSectionId[];
  hidden: RailSectionId[];
  /** The stored layout has arrived: what shows is the person's layout. */
  loaded: boolean;
  show(id: RailSectionId): void;
} {
  const panels = usePanelLayout();
  const { layout } = panels;
  const [draggingId, setDraggingId] = useState<RailSectionId | null>(null);
  const [dropTarget, setDropTarget] = useState<{ id: RailSectionId; side: "before" | "after" } | null>(null);
  const [groupDropTarget, setGroupDropTarget] = useState<PanelGroup | "remove" | null>(null);
  // The dividers and the Remove zone come in a tick after the drag starts:
  // WebKit (macOS's window) cancels a drag whose page changes during
  // `dragstart`, and the press becomes a text selection.
  const [showGroups, setShowGroups] = useState(false);
  const groupsTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const endDrag = useCallback(() => {
    if (groupsTimer.current !== null) clearTimeout(groupsTimer.current);
    groupsTimer.current = null;
    setDraggingId(null);
    setDropTarget(null);
    setGroupDropTarget(null);
    setShowGroups(false);
  }, []);
  useEffect(() => () => {
    if (groupsTimer.current !== null) clearTimeout(groupsTimer.current);
  }, []);

  const isExpanded = useCallback((id: RailSectionId) => !layout.collapsed.has(id), [layout]);

  const dragHandle = useCallback((id: RailSectionId) => ({
    draggable: true as const,
    onDragStart(e: React.DragEvent) {
      e.dataTransfer.setData(RAIL_SECTION_DRAG_MIME, id);
      e.dataTransfer.effectAllowed = "move";
      setDraggingId(id);
      if (groupsTimer.current !== null) clearTimeout(groupsTimer.current);
      groupsTimer.current = setTimeout(() => {
        groupsTimer.current = null;
        setShowGroups(true);
      }, 0);
    },
    onDragEnd: endDrag,
  }), [endDrag]);

  const groupDrop = useCallback((group: PanelGroup | null) => {
    // Safari accepts a drop only where `dragenter` was cancelled too.
    const accept = (e: React.DragEvent) => {
      if (!draggingId) return;
      if (!e.dataTransfer.types.includes(RAIL_SECTION_DRAG_MIME)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      const target = group ?? "remove";
      if (groupDropTarget !== target) setGroupDropTarget(target);
      if (dropTarget) setDropTarget(null);
    };
    return {
    onDragEnter: accept,
    onDragOver: accept,
    onDragLeave() {
      if (groupDropTarget === (group ?? "remove")) setGroupDropTarget(null);
    },
    onDrop(e: React.DragEvent) {
      e.preventDefault();
      const sourceId = e.dataTransfer.getData(RAIL_SECTION_DRAG_MIME) || draggingId;
      endDrag();
      if (!sourceId) return;
      if (group) panels.moveToGroup(sourceId, group);
      else panels.hide(sourceId);
    },
    };
  }, [draggingId, dropTarget, groupDropTarget, panels, endDrag]);

  const dropZone = useCallback((id: RailSectionId) => {
    // Only react to our own section drag. Accept it everywhere (even over
    // the dragged section itself) so the insertion line tracks the cursor
    // and the cursor resolves to "move" (not the "+" copy icon). Safari
    // accepts a drop only where `dragenter` was cancelled too.
    const accept = (e: React.DragEvent) => {
      if (!draggingId) return;
      if (!e.dataTransfer.types.includes(RAIL_SECTION_DRAG_MIME)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      const rect = e.currentTarget.getBoundingClientRect();
      const side: "before" | "after" = e.clientY < rect.top + rect.height / 2 ? "before" : "after";
      if (dropTarget?.id !== id || dropTarget.side !== side) setDropTarget({ id, side });
      if (groupDropTarget) setGroupDropTarget(null);
    };
    return {
    onDragEnter: accept,
    onDragOver: accept,
    onDragLeave() {
      if (dropTarget?.id === id) setDropTarget(null);
    },
    onDrop(e: React.DragEvent) {
      e.preventDefault();
      const sourceId = e.dataTransfer.getData(RAIL_SECTION_DRAG_MIME) || draggingId;
      endDrag();
      if (!sourceId || sourceId === id) return;
      const rect = e.currentTarget.getBoundingClientRect();
      const side = e.clientY >= rect.top + rect.height / 2 ? "after" : "before";
      panels.moveBeside(sourceId, id, side);
    },
    };
  }, [draggingId, dropTarget, groupDropTarget, panels, endDrag]);

  const dropSide = useCallback(
    (id: RailSectionId): "before" | "after" | null =>
      draggingId !== null && dropTarget?.id === id ? dropTarget.side : null,
    [dropTarget, draggingId],
  );

  const value = useMemo<RailSectionsValue>(
    () => ({
      isExpanded,
      setCollapsed: panels.setCollapsed,
      dragHandle,
      dropZone,
      dropSide,
      hide: panels.hide,
      groupOf: (id) => layout.groupOf.get(id),
      groups: panels.groups,
      moveToGroup: panels.moveToGroup,
    }),
    [isExpanded, panels, layout, dragHandle, dropZone, dropSide],
  );
  return {
    value,
    groups: layout.groups,
    groupList: panels.groups,
    dragging: draggingId !== null && showGroups,
    groupDrop,
    groupDropTarget,
    order: layout.order,
    hidden: layout.hidden,
    loaded: panels.loaded,
    show: panels.show,
  };
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
  opens,
  filled = false,
  hidden = false,
  morphing = false,
  onRestore,
  children,
}: {
  id: RailSectionId;
  title: string;
  count?: number;
  /** "danger" tints the title (Errors). */
  tone?: "danger";
  headerAction?: ReactNode;
  collapsedContent?: ReactNode;
  /** When set, the header ends with the open-page arrow (`OpenPageLink`,
   *  as a section heading's) to this content's full page. */
  opens?: { to: TabRef; name: string; onNavigate(ref: TabRef): void };
  /** Grown to the whole rail: it fills the rail's height, its chevron
   *  is the shrink button (`onRestore`), and it takes focus so Escape
   *  reaches the rail. */
  filled?: boolean;
  /** Out of sight while another panel fills the rail; still mounted, so
   *  it keeps its state, and still in the person's layout. */
  hidden?: boolean;
  /** A grow or shrink runs as a view transition: only then is the section
   *  named for it. A name makes it the box its fixed descendants place
   *  against, so at rest a row's menu would be clipped by the rail. */
  morphing?: boolean;
  onRestore?(): void;
  children: ReactNode;
}) {
  const ctx = useContext(RailSectionsContext);
  const expanded = filled || (ctx ? ctx.isExpanded(id) : true);
  const card = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (filled) card.current?.focus({ preventScroll: true });
  }, [filled]);
  const headerMenu = useContextMenu();
  const side = ctx ? ctx.dropSide(id) : null;
  const group = ctx?.groupOf(id);
  const shownIn = group === "thread" || group === "stream" ? group : null;
  const titleColor = tone === "danger" ? "var(--diff-del-fg, #f85149)" : "var(--text-primary)";
  return (
    // Wrapper holds the inter-pane margin + drop-zone and (unlike the card)
    // is not overflow-clipped, so the insertion line can sit in the gap.
    <div
      data-testid={`rail-section-${id}`}
      data-filled={filled ? "true" : undefined}
      {...(ctx ? ctx.dropZone(id) : {})}
      style={{
        position: "relative",
        margin: "0 6px 6px",
        // Don't let the flex column shrink panels when total height exceeds
        // the viewport — they stack and the column scrolls. A filled one
        // takes the column's height and scrolls inside.
        ...(filled ? { flex: "1 1 0", minHeight: 0, flexDirection: "column" as const } : { flexShrink: 0 }),
        display: hidden ? "none" : filled ? "flex" : undefined,
        ...(morphing ? { viewTransitionName: `rail-section-${id.replace(/[^A-Za-z0-9_-]/g, "-")}` } : {}),
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
        ref={card}
        tabIndex={filled ? -1 : undefined}
        style={{
          ...(filled ? { flex: 1, minHeight: 0, display: "flex", flexDirection: "column" as const, outline: "none" } : {}),
          // Inset, rounded card (matching the Search box) that recesses
          // below the lighter rail; the gap between cards reveals the rail
          // (IntelliJ-style grouping). Its edge is the strong one: the
          // card, header and all, stands off the rail.
          background: "var(--surface-card)",
          border: "1px solid var(--border-strong)",
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
          // A stronger wash than the chrome's: the rail around it wears the
          // 12% one over the card, so that would be the rail's own color and
          // disappear into it. This one is bluer than the rail and lighter
          // than the card below; the divider closes it off from the content.
          background: "linear-gradient(var(--rail-panel-header-bg), var(--rail-panel-header-bg)), var(--surface-card)",
          flexShrink: 0,
          borderBottom: expanded ? "1px solid var(--border-subtle)" : undefined,
        }}
        onContextMenu={(e) =>
          ctx
            ? headerMenu.open(e, [
                {
                  id: "panel.show-in",
                  label: "Show in",
                  enabled: true,
                  submenuHeader: "Where this panel shows: this thread, every thread of this stream, or every thread.",
                  submenu: ctx.groups.map((group) => ({
                    id: `panel.show-in.${group}`,
                    label: visibilityScopeLabel(group),
                    enabled: true,
                    checked: ctx.groupOf(id) === group,
                    run: () => ctx.moveToGroup(id, group),
                  })),
                },
                { id: "hide-panel", label: "Hide Panel", enabled: true, run: () => ctx.hide(id) },
              ])
            : undefined
        }
      >
        {/* A filled panel has nothing to be moved among: no drag handle. */}
        {filled ? null : (
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
        )}
        <button
          type="button"
          data-testid={`rail-section-toggle-${id}`}
          onClick={() => (filled ? onRestore?.() : ctx?.setCollapsed(id, expanded))}
          aria-expanded={expanded}
          title={filled ? "Shrink" : expanded ? "Collapse" : "Expand"}
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
            {filled ? <Minimize2 size={12} aria-hidden /> : expanded ? "▾" : "▸"}
          </span>
          <span>{title}</span>
          {shownIn ? (
            <span
              data-testid={`rail-section-shown-in-${id}`}
              title={shownIn === "thread" ? "Only in this thread" : "Only in this stream"}
              style={{ display: "inline-flex", color: "var(--text-muted)" }}
            >
              {shownIn === "thread" ? <MessageSquare size={11} aria-hidden /> : <Layers size={11} aria-hidden />}
            </span>
          ) : null}
          {count != null && count > 0 ? (
            <span style={{ color: "var(--text-muted)", fontSize: 11, fontWeight: 500 }}>{count}</span>
          ) : null}
        </button>
        {headerAction}
        {opens ? (
          <span
            data-testid={`rail-section-open-${id}`}
            onClick={(e) => e.stopPropagation()}
            style={{ display: "inline-flex", flexShrink: 0 }}
          >
            <OpenPageLink to={opens.to} name={opens.name} onNavigate={opens.onNavigate} />
          </span>
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
 */
export function RailHud({
  streamId,
  onOpenPage,
}: RailHudProps) {
  const width = useRailWidth();
  // One owner for every panel's page runs, shared with the bell and the
  // Alerts page (`PanelRunsProvider`).
  const { panels: allPanels, runs: panelRuns } = usePanelRuns();
  // A dock panel lives in the dock, not the rail.
  const extPanels = useMemo(() => allPanels.filter((p) => !p.dock), [allPanels]);
  const sections = useRailSections();
  const panelLayout = usePanelLayout();
  // A drag near the rail's top or bottom scrolls it, to reach what's out of view.
  const panelsBox = useRef<HTMLDivElement | null>(null);
  useDragAutoScroll(panelsBox, sections.dragging);
  const addMenu = useContextMenu();
  // The panel grown to the whole rail ("+N more"), if any: the window's
  // state only, never the person's layout. One at a time; gone with it.
  const [filledWanted, setFilled] = useState<RailSectionId | null>(null);
  const filled = filledWanted !== null && sections.order.includes(filledWanted) ? filledWanted : null;
  // While a grow or shrink transition runs, the sections carry their names.
  const [morphing, setMorphing] = useState(false);
  const fill = useCallback((id: RailSectionId | null) => withViewTransition(() => setFilled(id), setMorphing), []);

  // Every visible panel always renders (a stable list); each shows its own
  // empty state when it has no content.
  function renderSection(id: RailSectionId): ReactNode {
    const panel = extPanels.find((p) => extensionPanelId(p) === id);
    return panel ? (
      <ExtensionPanelSection
        key={id}
        panel={panel}
        runs={panelRuns[panel.id] ?? { body: null, badge: null, collapsed: null, count: null, choices: [] }}
        streamId={streamId ?? null}
        onOpenPage={onOpenPage}
        filled={filled === id}
        hidden={filled !== null && filled !== id}
        morphing={morphing}
        onFill={() => fill(id)}
        onRestore={() => fill(null)}
      />
    ) : null;
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
      onKeyDown={(e) => {
        // Escape puts the rail back from a filled panel.
        if (e.key !== "Escape" || filled === null || e.defaultPrevented) return;
        e.preventDefault();
        fill(null);
      }}
    >
      <div
        ref={panelsBox}
        data-testid="rail-panels"
        // A right-click on the rail's own space (not a panel) lists every
        // panel, checked when shown: View ▸ Sidebar Panels's list.
        onContextMenu={(e) => {
          if (e.target !== e.currentTarget || filled !== null) return;
          addMenu.open(e, railPanelItems(panelLayout.entries, panelLayout.toggle));
        }}
        style={{ flex: 1, overflow: filled ? "hidden" : "auto", display: "flex", flexDirection: "column", minHeight: 0 }}
      >
        <RailSectionsContext.Provider value={sections.value}>
          {sections.groupList.map((group) => (
            <Fragment key={group}>
              {sections.dragging ? (
                <GroupDivider
                  group={group}
                  over={sections.groupDropTarget === group}
                  {...sections.groupDrop(group)}
                />
              ) : null}
              {sections.groups[group].map((id) => renderSection(id))}
            </Fragment>
          ))}
          {sections.dragging ? (
            <div
              data-testid="rail-remove-zone"
              {...sections.groupDrop(null)}
              style={{
                margin: "2px 6px 6px",
                padding: "10px 6px",
                textAlign: "center",
                fontSize: 11,
                color: "var(--text-muted)",
                borderRadius: 6,
                border: `1px dashed ${sections.groupDropTarget === "remove" ? "var(--accent)" : "var(--border-subtle)"}`,
                boxShadow: sections.groupDropTarget === "remove" ? "0 0 0 2px color-mix(in srgb, var(--accent) 30%, transparent)" : undefined,
              }}
            >
              Remove from the rail
            </div>
          ) : null}
        </RailSectionsContext.Provider>
        {addMenu.menu}
      </div>
      <RailResizeHandle onChange={width.setFromDelta} />
    </aside>
  );
}

/** A group's divider while a panel is dragged: its name, and a drop
 *  target that moves the panel to the group's end. */
function GroupDivider({
  group,
  over,
  ...drop
}: {
  group: PanelGroup;
  over: boolean;
  onDragEnter(e: React.DragEvent): void;
  onDragOver(e: React.DragEvent): void;
  onDragLeave(): void;
  onDrop(e: React.DragEvent): void;
}) {
  return (
    <div
      data-testid={`rail-group-divider-${group}`}
      {...drop}
      style={{
        margin: "0 6px 6px",
        padding: "4px 6px",
        display: "flex",
        alignItems: "center",
        gap: 6,
        fontSize: 10,
        fontWeight: 600,
        textTransform: "uppercase",
        letterSpacing: 0.4,
        color: over ? "var(--accent-fg)" : "var(--text-muted)",
        borderRadius: 6,
        border: `1px dashed ${over ? "var(--accent)" : "transparent"}`,
        boxShadow: over ? "0 0 0 2px color-mix(in srgb, var(--accent) 30%, transparent)" : undefined,
      }}
    >
      <span style={{ flex: 1, height: 1, background: "var(--border-subtle)" }} />
      {visibilityScopeLabel(group)}
      <span style={{ flex: 1, height: 1, background: "var(--border-subtle)" }} />
    </div>
  );
}

/** Run a change to what the rail shows as a view transition where the
 *  webview has them (the panel grows or shrinks into place); at once
 *  elsewhere. */
/** Runs `change` as a view transition where the webview has one, at once
 *  elsewhere; `naming` is on for its length (the old and new snapshots
 *  both need the sections' names). */
function withViewTransition(change: () => void, naming: (on: boolean) => void) {
  const doc = document as Document & { startViewTransition?(update: () => void): { finished: Promise<unknown> } };
  if (typeof doc.startViewTransition !== "function") {
    change();
    return;
  }
  flushSync(() => naming(true));
  const transition = doc.startViewTransition(() => flushSync(change));
  void transition.finished.finally(() => naming(false));
}

const RAIL_MIN_WIDTH = 260;
const RAIL_MAX_WIDTH = 600;

function useRailWidth() {
  const [stored, setValue] = useAppPreference<number>("railWidth", RAIL_MIN_WIDTH);
  const value = Number.isFinite(stored) ? clampRailWidth(stored) : RAIL_MIN_WIDTH;
  const startRef = useRef<{ start: number; base: number } | null>(null);
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

/** Muted empty-state line for a section that has no content but still
 *  renders (the pane list is stable — sections never disappear). */
function RailEmpty({ label }: { label: string }) {
  return (
    <div style={{ padding: "4px 14px 10px" }}>
      <EmptyState compact title={label} />
    </div>
  );
}

/** An extension's panel (P6.G1): its body page, compact with its buttons; its count in the
 *  header (the count page's, else the badge's while it fires); and,
 *  collapsed, its collapsed page compact as the summary. */
function ExtensionPanelSection({
  panel,
  runs,
  streamId,
  onOpenPage,
  filled,
  hidden,
  morphing,
  onFill,
  onRestore,
}: {
  panel: ExtensionPanel;
  /** The panel's runs, from the rail's one owner (`useExtensionPanelRuns`). */
  runs: PanelRuns;
  streamId: string | null;
  onOpenPage(ref: TabRef): void;
  /** Grown to the whole rail: its body every row, scrolling. */
  filled: boolean;
  /** Another panel fills the rail. */
  hidden: boolean;
  /** A grow or shrink transition is running. */
  morphing: boolean;
  onFill(): void;
  onRestore(): void;
}) {
  const { choose } = usePanelRuns();
  const extensions = useExtensions();
  // The value in force is the one the body ran with (the daemon's).
  const toggles = runs.choices;
  // A panel's part: its page drawn small. The body keeps its buttons and
  // its "+N more" asks for the next size up: in the rail it grows the panel
  // to the whole rail, where it scrolls; grown (where only a list that caps
  // its groups still has one) it opens the panel's page. The collapsed
  // line is stats.
  const openRef = panelOpenRef(panel, extensions);
  const body = runs.body ? (
    <div
      data-testid="rail-panel-body"
      className="ox-compact"
      style={{ padding: "4px 10px 8px", fontSize: "var(--text-xs)", ...(filled ? { flex: 1, minHeight: 0, overflowY: "auto" as const } : {}) }}
    >
      <PageTree run={runs.body.run} runId={runs.body.runId} compact partButtons uncapped={filled} onMore={filled ? () => onOpenPage(openRef) : onFill} streamId={streamId} onOpenPage={onOpenPage} />
    </div>
  ) : null;
  const collapsed = runs.collapsed ? (
    <div data-testid="rail-panel-collapsed" className="ox-compact" style={{ padding: "4px 10px 8px", fontSize: "var(--text-xs)" }}>
      <PageTree run={runs.collapsed.run} runId={runs.collapsed.runId} compact partButtons={false} streamId={streamId} onOpenPage={onOpenPage} />
    </div>
  ) : undefined;
  return (
    <RailSection
      id={extensionPanelId(panel)}
      title={panel.title}
      filled={filled}
      hidden={hidden}
      morphing={morphing}
      onRestore={onRestore}
      count={runs.count ?? undefined}
      collapsedContent={collapsed}
      opens={{ to: openRef, name: panel.title, onNavigate: onOpenPage }}
      headerAction={
        toggles.length > 0 ? (
          <>
            {toggles.map((c) => (
              <PanelChoiceToggle key={c.name} choice={c} onPick={(v) => choose(panel.id, c.name, v)} />
            ))}
          </>
        ) : undefined
      }
    >
      {body ? (
        body
      ) : (
        <div style={{ padding: "4px 10px 8px", fontSize: "var(--text-xs)" }}>
          <RailEmpty label="Loading…" />
        </div>
      )}
    </RailSection>
  );
}

/** A panel's choice param in its header: one small button per
 *  option, the one in force pressed. */
function PanelChoiceToggle({ choice, onPick }: { choice: PanelChoice; onPick(value: string): void }) {
  return (
    <span role="group" aria-label={choice.label ?? choice.name} style={{ display: "inline-flex", gap: 2 }}>
      {choice.options.map((o) => {
        const active = o.value === choice.value;
        // An option that names an icon is the icon alone, its label the
        // hover and the button's name.
        const Icon = iconFor(o.icon);
        return (
          <button
            key={o.value}
            type="button"
            data-testid={`rail-panel-choice-${choice.name}-${o.value}`}
            aria-pressed={active}
            title={Icon ? o.label : undefined}
            aria-label={Icon ? o.label : undefined}
            onClick={(e) => {
              e.stopPropagation();
              if (!active) onPick(o.value);
            }}
            style={{
              // The one in force is boxed in the accent over a faint
              // accent wash, so it reads as picked against the header.
              background: active ? "var(--rail-panel-choice-bg)" : "transparent",
              border: `1px solid ${active ? "var(--accent)" : "transparent"}`,
              borderRadius: 4,
              color: active ? "var(--text-primary)" : "var(--text-secondary)",
              cursor: active ? "default" : "pointer",
              fontSize: 10,
              padding: Icon ? "2px 3px" : "0 4px",
              display: "inline-flex",
              alignItems: "center",
            }}
          >
            {Icon ? <Icon size={13} aria-hidden /> : o.label}
          </button>
        );
      })}
    </span>
  );
}

