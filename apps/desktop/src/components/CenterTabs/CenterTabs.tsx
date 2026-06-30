import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { AgentStatus } from "../../api.js";
import { kindForTabId, PageKindIcon } from "../../pageKinds.js";
import { AgentStatusDot } from "../AgentStatusDot.js";
import { useContextMenu } from "../useRowContextMenu.js";
import type { MenuItem } from "../../menu.js";
import { ErrorBoundary } from "../ErrorBoundary.js";
import { leadingPinnedCount, moveToIndex, reorderToAfterPinned } from "./centerTabsReorder.js";
import { tabsToCloseOthers, tabsToCloseRight } from "./tabClose.js";

export interface CenterTab {
  id: string;
  label: string;
  closable: boolean;
  render: () => ReactNode;
  agentStatus?: AgentStatus;
  /** Kind-specific right-click menu entries for this tab. They render at
   *  the top of the tab's context menu, above the universal "Close Other
   *  Tabs" / "Close Tabs to the Right" pair that CenterTabs appends for
   *  every tab. (Right-click is the per-row action surface project-wide;
   *  there is no visible kebab — see `.context/usability.md`.) */
  contextMenu?: MenuItem[];
  /** When true, this tab does NOT appear in the strip but its body
   *  still mounts (kept hidden via display:none). Used by the host
   *  to keep back/forward stack entries alive so navigation between
   *  them preserves React component state without remounting. */
  hidden?: boolean;
}

interface CenterTabsProps {
  tabs: CenterTab[];
  activeId: string;
  onActivate(id: string): void;
  onClose?(id: string): void;
  /** Rendered above the active tab's content. */
  header?: ReactNode;
  /** Called when the user drag-reorders tabs. Receives the new full id list. */
  onReorder?(orderedIds: string[]): void;
}

const TAB_DRAG_MIME = "application/x-oxplow-center-tab";

export function CenterTabs({ tabs, activeId, onActivate, onClose, header, onReorder }: CenterTabsProps) {
  const active = tabs.find((t) => t.id === activeId) ?? tabs.find((t) => !t.hidden) ?? tabs[0] ?? null;
  const stripTabs = tabs.filter((t) => !t.hidden);
  const cm = useContextMenu();
  const [hoverId, setHoverId] = useState<string | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  // Where the dragged tab would land: an insertion line drawn on the
  // `before` (left) or `after` (right) edge of `id`, chosen by which
  // half of that tab the cursor is over.
  const [dropTarget, setDropTarget] = useState<{ id: string; side: "before" | "after" } | null>(null);
  const draggingTab = draggingId ? tabs.find((t) => t.id === draggingId) ?? null : null;
  const stripScrollRef = useRef<HTMLDivElement>(null);
  // The outer flex container that wraps the strip + overflow button.
  // We measure against its width (stable) rather than the strip's
  // own width — the strip shrinks once we hide a tab, which would
  // otherwise cascade into "every tab is too wide" and hide them all.
  const outerBarRef = useRef<HTMLDivElement>(null);
  const overflowButtonRef = useRef<HTMLButtonElement>(null);
  const [overflowOpen, setOverflowOpen] = useState(false);
  // Tabs whose natural right edge would extend past the strip's
  // visible area. We render them `display: none` in the strip so the
  // user never sees a half-clipped chip — they're still listed in the
  // overflow panel. Tab widths are cached so a freshly-hidden tab
  // doesn't lose its width when re-measurement runs (its offsetWidth
  // becomes 0 once display:none).
  const widthCacheRef = useRef<Map<string, number>>(new Map());
  const [hiddenInStripIds, setHiddenInStripIds] = useState<Set<string>>(new Set());
  const hasOverflow = hiddenInStripIds.size > 0;

  // A tab is reorderable when reordering is wired and the tab isn't
  // pinned. "Pinned" = non-closable (the Agent tab); it stays at the
  // front and is never a drag source or drop target. Every other tab
  // can be dragged anywhere in the strip — there are no per-kind groups.
  const isReorderable = (tab: CenterTab): boolean => !!onReorder && tab.closable;

  // Close a batch of tabs through the host's single-tab close path, then —
  // only if the close swept away the active tab — fall the selection back to
  // the anchor the user right-clicked (so the strip doesn't snap to Agent
  // when a perfectly good neighbour is still open).
  const closeMany = (ids: string[], anchorId: string): void => {
    if (!onClose || ids.length === 0) return;
    const closingActive = ids.includes(activeId);
    for (const id of ids) onClose(id);
    if (closingActive) onActivate(anchorId);
  };

  // Per-tab right-click menu: the tab's own entries (if any) followed by the
  // universal "Close Other Tabs" / "Close Tabs to the Right" pair. The pair
  // only appears when the host wired a close handler; each is disabled when it
  // has no targets. Targets are computed over the real open tabs (`stripTabs`)
  // so overflowed-but-open tabs still close while back/forward stack entries
  // (`hidden`) are excluded.
  const buildTabMenu = (tab: CenterTab): MenuItem[] => {
    const items: MenuItem[] = tab.contextMenu ? [...tab.contextMenu] : [];
    if (onClose) {
      const others = tabsToCloseOthers(stripTabs, tab.id);
      const toRight = tabsToCloseRight(stripTabs, tab.id);
      if (items.length > 0) {
        items.push({ id: "tab.close-sep", label: "", enabled: false, separator: true });
      }
      items.push({
        id: "tab.close-others",
        label: "Close Other Tabs",
        enabled: others.length > 0,
        run: () => closeMany(others, tab.id),
      });
      items.push({
        id: "tab.close-right",
        label: "Close Tabs to the Right",
        enabled: toRight.length > 0,
        run: () => closeMany(toRight, tab.id),
      });
    }
    return items;
  };

  useLayoutEffect(() => {
    const outer = outerBarRef.current;
    const strip = stripScrollRef.current;
    if (!outer || !strip) return;
    const OVERFLOW_BUTTON_RESERVE = 36; // matches button minWidth below
    const measure = () => {
      // Update width cache from currently-visible tab chips.
      const children = Array.from(strip.children) as HTMLElement[];
      for (const child of children) {
        const id = child.dataset.tabId;
        if (!id) continue;
        const w = child.offsetWidth;
        if (w > 0) widthCacheRef.current.set(id, w);
      }
      // Drop cache entries for tabs that no longer exist.
      const liveIds = new Set(stripTabs.map((t) => t.id));
      for (const id of widthCacheRef.current.keys()) {
        if (!liveIds.has(id)) widthCacheRef.current.delete(id);
      }
      // Two-pass fit: first without reserving the overflow-button
      // slot. If everything fits, no button needed. Otherwise retry
      // with the reserve so the button has room.
      const outerWidth = outer.clientWidth;
      const computeHidden = (reserve: number): Set<string> => {
        const avail = outerWidth - reserve;
        const hidden = new Set<string>();
        let used = 0;
        for (const tab of stripTabs) {
          const w = widthCacheRef.current.get(tab.id);
          if (w == null) continue; // not yet measured — assume fits
          if (used + w > avail) {
            hidden.add(tab.id);
          } else {
            used += w;
          }
        }
        return hidden;
      };
      let next = computeHidden(0);
      if (next.size > 0) next = computeHidden(OVERFLOW_BUTTON_RESERVE);
      setHiddenInStripIds((prev) => {
        if (prev.size === next.size && [...prev].every((id) => next.has(id))) return prev;
        return next;
      });
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(outer);
    for (const child of Array.from(strip.children)) ro.observe(child);
    return () => ro.disconnect();
  }, [stripTabs]);

  useEffect(() => {
    if (!hasOverflow && overflowOpen) setOverflowOpen(false);
  }, [hasOverflow, overflowOpen]);

  // Promote a hidden tab into the reorderable region (just after the
  // agent tab and any other pinned non-reorderable tabs). Returns true
  // when a reorder was dispatched.
  const promoteHiddenIntoStrip = (id: string): boolean => {
    if (!onReorder) return false;
    if (!hiddenInStripIds.has(id)) return false;
    const ids = tabs.map((t) => t.id);
    // Land it right after the leading run of pinned (non-closable) tabs
    // — i.e. directly after Agent — so it surfaces in the most prominent
    // slot rather than wherever the first file tab happens to be.
    const pinned = leadingPinnedCount(tabs.map((t) => t.closable));
    const next = reorderToAfterPinned(ids, pinned, id);
    if (next === ids) return false;
    onReorder(next);
    return true;
  };

  // Any activation that lands on a tab currently cut off from the strip
  // (left nav click, palette, programmatic switch, etc.) should surface
  // the tab by reordering it just past the pinned agent tab. The
  // overflow-panel click path uses the same helper inline.
  useEffect(() => {
    if (!activeId) return;
    if (!hiddenInStripIds.has(activeId)) return;
    promoteHiddenIntoStrip(activeId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeId, hiddenInStripIds]);
  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
      <div ref={outerBarRef} style={{ display: "flex", borderBottom: "1px solid var(--border-strong)", background: "linear-gradient(var(--panel-header-bg), var(--panel-header-bg)), var(--surface-card)", minHeight: 36, position: "relative" }}>
        <div ref={stripScrollRef} style={{ display: "flex", flex: "0 0 auto", maxWidth: "100%", minWidth: 0, overflow: "hidden" }}>
        {stripTabs.map((tab) => {
          const isActive = tab.id === active?.id;
          const isHover = !isActive && hoverId === tab.id;
          const canDrag = isReorderable(tab);
          // Insertion line on this tab's left/right edge when a drag is
          // hovering it. Shown even when this is the dragged tab itself
          // (so the line tracks the cursor across the whole strip).
          const dropSide =
            !!draggingTab && isReorderable(tab) && dropTarget?.id === tab.id ? dropTarget.side : null;
          const hiddenInStrip = hiddenInStripIds.has(tab.id);
          return (
            <div
              key={tab.id}
              data-testid={`center-tab-${tab.id}`}
              data-tab-id={tab.id}
              draggable={canDrag}
              tabIndex={0}
              onClick={() => onActivate(tab.id)}
              onContextMenu={(e) => cm.open(e, buildTabMenu(tab))}
              onKeyDown={(e) => cm.openForKey(e, buildTabMenu(tab))}
              onMouseEnter={() => setHoverId(tab.id)}
              onMouseLeave={() => setHoverId((prev) => (prev === tab.id ? null : prev))}
              onDragStart={canDrag ? (event) => {
                event.dataTransfer.setData(TAB_DRAG_MIME, tab.id);
                event.dataTransfer.effectAllowed = "move";
                setDraggingId(tab.id);
              } : undefined}
              onDragEnd={canDrag ? () => {
                setDraggingId(null);
                setDropTarget(null);
              } : undefined}
              onDragOver={onReorder ? (event) => {
                if (!draggingTab) return;
                // Any reorderable tab is a valid drop target (no per-kind
                // groups); pinned tabs are skipped so nothing lands
                // before Agent. Which half of the tab the cursor is over
                // decides whether the line sits before or after it.
                if (!isReorderable(tab)) return;
                event.preventDefault();
                event.dataTransfer.dropEffect = "move";
                const rect = event.currentTarget.getBoundingClientRect();
                const side: "before" | "after" =
                  event.clientX < rect.left + rect.width / 2 ? "before" : "after";
                if (dropTarget?.id !== tab.id || dropTarget.side !== side) {
                  setDropTarget({ id: tab.id, side });
                }
              } : undefined}
              onDragLeave={onReorder ? () => {
                if (dropTarget?.id === tab.id) setDropTarget(null);
              } : undefined}
              onDrop={onReorder ? (event) => {
                if (!draggingTab) return;
                if (!isReorderable(tab)) return;
                event.preventDefault();
                const sourceId = draggingTab.id;
                setDraggingId(null);
                setDropTarget(null);
                const ids = tabs.map((t) => t.id);
                const targetIdx = ids.indexOf(tab.id);
                if (targetIdx < 0) return;
                const rect = event.currentTarget.getBoundingClientRect();
                const after = event.clientX >= rect.left + rect.width / 2;
                // desiredIndex is the slot in the ORIGINAL order; moveToIndex
                // handles the source-removal shift and no-ops in place.
                const next = moveToIndex(ids, sourceId, after ? targetIdx + 1 : targetIdx);
                if (next !== ids) onReorder(next);
              } : undefined}
              style={{
                position: "relative",
                padding: "10px 14px",
                background: isActive
                  ? "var(--accent-soft-bg)"
                  : isHover
                    ? "rgba(255, 255, 255, 0.04)"
                    : "transparent",
                color: isActive ? "var(--accent)" : "var(--text-secondary)",
                borderRight: "1px solid var(--border-strong)",
                // The content panel's own border draws the strip's top and
                // left edges; tabs only carry their inter-tab dividers and
                // the active frame.
                borderTop: "1px solid transparent",
                borderLeft: isActive ? "1px solid var(--border-strong)" : "1px solid transparent",
                borderBottom: isActive ? "3px solid var(--accent)" : "3px solid transparent",
                opacity: draggingId === tab.id ? 0.5 : 1,
                cursor: canDrag ? "grab" : "pointer",
                fontSize: "var(--text-sm)",
                fontWeight: isActive ? 600 : 400,
                display: hiddenInStrip ? "none" : "inline-flex",
                alignItems: "center",
                gap: 6,
              }}
            >
              {dropSide ? (
                <span
                  aria-hidden
                  data-testid={`center-tab-drop-line-${tab.id}-${dropSide}`}
                  style={{
                    position: "absolute",
                    top: 2,
                    bottom: 2,
                    [dropSide === "before" ? "left" : "right"]: -1,
                    width: 3,
                    background: "var(--accent)",
                    borderRadius: 2,
                    pointerEvents: "none",
                    zIndex: 1,
                  }}
                />
              ) : null}
              {tab.agentStatus ? <AgentStatusDot status={tab.agentStatus} /> : null}
              <PageKindIcon
                kind={kindForTabId(tab.id)}
                size={13}
                style={{ color: "var(--text-secondary)", flexShrink: 0 }}
              />
              <span
                title={tab.label}
                style={{
                  maxWidth: 180,
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  display: "inline-block",
                  verticalAlign: "middle",
                }}
              >
                {tab.label}
              </span>
              {tab.closable && onClose ? (
                <button type="button"
                  data-testid={`center-tab-close-${tab.id}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    onClose(tab.id);
                  }}
                  title={`Close ${tab.label}`}
                  style={{
                    border: "none",
                    background: "transparent",
                    color: "var(--muted)",
                    cursor: "pointer",
                    padding: "0 2px",
                    fontSize: "var(--text-base)",
                    lineHeight: 1,
                  }}
                >
                  ×
                </button>
              ) : null}
            </div>
          );
        })}
        </div>
        {hasOverflow ? (
          <button
            ref={overflowButtonRef}
            type="button"
            data-testid="center-tabs-overflow-button"
            aria-label="Show all tabs"
            title="Show all tabs"
            onClick={(e) => {
              e.stopPropagation();
              setOverflowOpen((v) => !v);
            }}
            style={{
              flex: "1 1 auto",
              minWidth: 36,
              alignSelf: "stretch",
              padding: "0 10px",
              border: "none",
              borderLeft: "1px solid var(--border-strong)",
              background: "var(--surface-tab-inactive)",
              color: "var(--text-secondary)",
              cursor: "pointer",
              fontSize: 24,
              lineHeight: 1,
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "flex-start",
            }}
          >
            ▾
          </button>
        ) : null}
        {overflowOpen ? (
          <OverflowPanel
            tabs={stripTabs}
            activeId={active?.id ?? null}
            anchorRef={overflowButtonRef}
            onActivate={(id) => {
              setOverflowOpen(false);
              // Promotion also runs via the activeId effect below,
              // but doing it here keeps the click→reorder→activate
              // sequence in a single React batch.
              promoteHiddenIntoStrip(id);
              onActivate(id);
            }}
            onClose={onClose}
            onDismiss={() => setOverflowOpen(false)}
          />
        ) : null}
      </div>
      {header}
      <div style={{ flex: 1, minHeight: 0, minWidth: 0, overflow: "hidden", display: "flex", flexDirection: "column", position: "relative" }}>
        {/* Render every tab body as a sibling, only the active one
         *  visible. Hidden tabs (back/forward stack entries the host
         *  pushes for each slot) stay mounted so navigating back to
         *  them preserves their React state — scroll position,
         *  expanded trees, draft text, etc. */}
        {tabs.map((tab) => {
          const isActive = tab.id === active?.id;
          return (
            <div
              key={tab.id}
              style={{
                display: isActive ? "flex" : "none",
                flex: 1,
                minHeight: 0,
                minWidth: 0,
                flexDirection: "column",
              }}
              data-testid={`center-tab-body-${tab.id}`}
              aria-hidden={!isActive}
            >
              <ErrorBoundary label={tab.label}>
                {tab.render()}
              </ErrorBoundary>
            </div>
          );
        })}
      </div>
      {cm.menu}
    </div>
  );
}

interface OverflowPanelProps {
  tabs: CenterTab[];
  activeId: string | null;
  anchorRef: React.RefObject<HTMLButtonElement | null>;
  onActivate(id: string): void;
  onClose?(id: string): void;
  onDismiss(): void;
}

function OverflowPanel({ tabs, activeId, anchorRef, onActivate, onClose, onDismiss }: OverflowPanelProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; right: number } | null>(null);

  useLayoutEffect(() => {
    const anchor = anchorRef.current;
    if (!anchor) return;
    const rect = anchor.getBoundingClientRect();
    setPos({ top: rect.bottom + 2, right: Math.max(8, window.innerWidth - rect.right) });
  }, [anchorRef]);

  useEffect(() => {
    function onPointerDown(e: MouseEvent) {
      if (rootRef.current?.contains(e.target as Node)) return;
      if (anchorRef.current?.contains(e.target as Node)) return;
      onDismiss();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onDismiss();
    }
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [anchorRef, onDismiss]);

  if (!pos) return null;
  return (
    <div
      ref={rootRef}
      data-testid="center-tabs-overflow-panel"
      style={{
        position: "fixed",
        top: pos.top,
        right: pos.right,
        zIndex: 1000,
        background: "var(--surface-card)",
        border: "1px solid var(--border-strong)",
        boxShadow: "0 6px 20px rgba(0,0,0,0.25)",
        minWidth: 240,
        maxWidth: 360,
        maxHeight: "60vh",
        overflowY: "auto",
        padding: "4px 0",
      }}
    >
      {tabs.map((tab) => {
        const isActive = tab.id === activeId;
        return (
          <div
            key={tab.id}
            data-testid={`center-tabs-overflow-item-${tab.id}`}
            onClick={() => onActivate(tab.id)}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 6,
              padding: "6px 10px",
              cursor: "pointer",
              background: isActive ? "var(--surface-tab-active)" : "transparent",
              color: isActive ? "var(--accent)" : "var(--text-primary)",
              fontWeight: isActive ? 600 : 400,
              fontSize: "var(--text-sm)",
            }}
            onMouseEnter={(e) => {
              if (!isActive) (e.currentTarget as HTMLDivElement).style.background = "var(--surface-hover)";
            }}
            onMouseLeave={(e) => {
              if (!isActive) (e.currentTarget as HTMLDivElement).style.background = "transparent";
            }}
          >
            {tab.agentStatus ? <AgentStatusDot status={tab.agentStatus} /> : null}
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={tab.label}>
              {tab.label}
            </span>
            {tab.closable && onClose ? (
              <button
                type="button"
                data-testid={`center-tabs-overflow-close-${tab.id}`}
                onClick={(e) => {
                  e.stopPropagation();
                  onClose(tab.id);
                }}
                title={`Close ${tab.label}`}
                style={{
                  border: "none",
                  background: "transparent",
                  color: "var(--muted)",
                  cursor: "pointer",
                  padding: "0 2px",
                  fontSize: "var(--text-base)",
                  lineHeight: 1,
                }}
              >
                ×
              </button>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
