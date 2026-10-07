import { LensSlots } from "../../lens/LensSlots.js";
import { numericRowId } from "../../lens/lensModel.js";
import type { CSSProperties } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { AgentStatus, Thread } from "../../api.js";
import {
  CANONICAL_STATES,
  type CanonicalState,
  type FieldDecl,
  type WorkItem,
  type WorkList,
  type WorkListProfile,
} from "../../workItems.js";
import {
  listOpenAgentTurns,
  type OpenAgentTurn,
  removeFollowup,
  subscribeAgentTurns,
} from "../../api.js";
import { decodeWorkItemDrag, dragHasWorkItems } from "../../agent-context-dnd.js";
import { WORK_ITEM_DRAG_MIME } from "../../dragMimes.js";
import { ContextMenu } from "../ContextMenu.js";
import { showToast } from "../toastStore.js";
import type { MenuItem } from "../../menu.js";
import { runWithError } from "../../ui-error.js";
import { insertIntoAgent } from "../../agent-input-bus.js";
import { formatContextMention } from "../../agent-context-ref.js";
import { requestCommentCompose } from "../../comment-compose-bus.js";
import { composeForElement } from "../Comments/useDomAnnotations.js";
import { SelectionActionBar } from "./SelectionActionBar.js";
import { SectionHeaderMenu, TaskGroupList } from "./TaskGroupList.js";
import type { TaskDetailChanges } from "./TaskDetail.js";
import {
  applyStateFilter,
  buildBacklogGroups,
  buildGroups,
  classifyState,
  statusLabel,
  useCollapsedSections,
  type TaskSectionKind,
} from "./plan-utils.js";

const STATUS_RANK: Record<string, number> = { inProgress: 0, ready: 1, blocked: 2, done: 3 };
function statusOrderRank(state: CanonicalState): number {
  return STATUS_RANK[classifyState(state)] ?? 0;
}

/** A keyboard picker: the state, or one of the list's own fields. */
type KbPickerKind = { kind: "state" } | { kind: "field"; field: FieldDecl };

const mention = (item: WorkItem) =>
  formatContextMention({ kind: "work_item", ref: item.ref, title: item.title, state: item.state });

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === "TEXTAREA") return true;
  if (tag === "INPUT") {
    const type = (target as HTMLInputElement).type;
    return type === "text" || type === "search" || type === "email" || type === "url" || type === "password" || type === "" || type === "tel";
  }
  return false;
}

interface Props {
  thread: Thread | null;
  activeThreadId: string | null;
  threadWork: WorkList | null;
  /** Live agent status for the displayed thread. Drives the In Progress
   *  empty-state placeholder ("Thinking..." vs "Waiting"). */
  agentStatus?: AgentStatus;
  backlog: WorkList | null;
  /** The active list: what it can do (each action shows only when it
   *  can) and its own fields. */
  profile: WorkListProfile;
  /** Shorthand for `profile.fields`. */
  fields: FieldDecl[];
  onUpdateTask(ref: string, changes: TaskDetailChanges): Promise<void>;
  onDeleteTask(ref: string): Promise<void>;
  /** Apply a drag's new order to the thread's list (refs). */
  onReorderTasks(orderedRefs: string[]): Promise<void>;
  onReorderBacklog(orderedRefs: string[]): Promise<void>;
  onMoveItemToBacklog(ref: string, fromThreadId: string): Promise<void>;
  openNewRequest?: number;
  /** Open the page for the given item. Change the token to request again
   *  even if the ref repeats. */
  editRequest?: { ref: string; token: number } | null;
  /** On mount, PlanPane calls this with its openCreateModal function so
   *  the parent can open the New-Task modal imperatively — used for
   *  menu-click dispatches where React's effect scheduler can stall. */
  registerOpenCreate?(fn: () => void): void;
  /** Route the "new task" / "+ Task on epic" buttons to a NewTaskPage
   *  tab. When omitted, those buttons do nothing (tests, standalone
   *  usages). */
  onOpenNewTaskPage?(payload: { parentRef?: string | null }): void;
  /** Route a row click / Enter to the item's page. When omitted, row
   *  clicks still select but no page opens. */
  onOpenTaskPage?(ref: string): void;
  /** Restrict the visible sections (Ready / Blocked / etc). Used by the
   *  page split: Plan Work shows ready+blocked+done previews,
   *  Done Work / Archived show only "done", etc. Default = all four. */
  visibleSections?: TaskSectionKind[];
  /** Cap the number of items rendered per section after sort. Used by
   *  Plan Work to render "last 5" previews of Done. */
  sectionItemLimit?: Partial<Record<TaskSectionKind, number>>;
  /** Filter items by state before grouping (the Tasks list leaves out
   *  canceled ones). */
  onlyStates?: CanonicalState[];
  excludeStates?: CanonicalState[];
  /** Per-section header link nodes (right-aligned, after `sectionActions`).
   *  Used by Plan Work for "View all done →" links pointing at the
   *  dedicated Done Work / Archived pages. */
  extraSectionLinks?: Partial<Record<TaskSectionKind, React.ReactNode>>;
  /** Pin the pane mode and disable the bottom-bar toggle. The Backlog
   *  page passes `"backlog"` so the pane renders the stream-global
   *  backlog full-pane. */
  forceMode?: "thread" | "backlog";
  /** Suppress the bottom Backlog chip entirely. The page split drops it
   *  in favour of rail-nav + a "View backlog" link on Plan Work. */
  hideBacklogChip?: boolean;
}

interface ContextMenuState {
  x: number;
  y: number;
  item: WorkItem;
  /** Non-null when the right-clicked item belongs to a multi-selection. */
  groupIds: string[] | null;
}

export function PlanPane({
  thread,
  activeThreadId,
  threadWork,
  agentStatus,
  backlog,
  profile,
  fields,
  onUpdateTask,
  onDeleteTask,
  onReorderTasks,
  onReorderBacklog,
  onMoveItemToBacklog,
  openNewRequest,
  editRequest,
  registerOpenCreate,
  onOpenNewTaskPage,
  onOpenTaskPage,
  visibleSections,
  sectionItemLimit,
  onlyStates,
  excludeStates,
  extraSectionLinks,
  forceMode,
  hideBacklogChip = false,
}: Props) {
  const features = profile.features;
  const editableEnums = fields.filter((f) => f.kind === "enum" && !f.read_only);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [internalMode, setInternalMode] = useState<"thread" | "backlog">("thread");
  const mode = forceMode ?? internalMode;
  const [backlogChipDragOver, setBacklogChipDragOver] = useState(false);
  const { isCollapsed: isSectionCollapsed, toggle: onToggleSectionCollapsed } = useCollapsedSections();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // Extra "marked" ids for multi-select beyond the primary `selectedId`. Driven
  // by Cmd/Ctrl+click (toggle) and Shift+click (range from selectedId). When a
  // drag starts on any of the effectiveMarkedIds, the drag payload carries the
  // whole set so drop targets — the backlog chip, task rows and group headers
  // in `TaskGroupList`, and the agent terminal — can move them all in one
  // gesture. Plain click clears marks.
  const [markedIds, setMarkedIds] = useState<Set<string>>(() => new Set());
  const [kbPicker, setKbPicker] = useState<(KbPickerKind & { itemId: string; extraIds?: string[] }) | null>(null);
  const paneRef = useRef<HTMLDivElement | null>(null);

  const threadId = thread?.id ?? null;
  const streamId = thread?.stream_id ?? null;

  // Open agent turns (ended_at IS NULL) render as live spinner rows at
  // the top of the In Progress section — the passive "agent is doing
  // something right now" affordance CLAUDE.md describes. Seeded per
  // thread, refreshed on every commit naming `v_agent_turn`, and
  // emptied when the Stop hook closes the turn.
  const [openTurns, setOpenTurns] = useState<OpenAgentTurn[]>([]);
  useEffect(() => {
    if (!threadId) {
      setOpenTurns([]);
      return;
    }
    let cancelled = false;
    const refresh = () => {
      void listOpenAgentTurns(threadId)
        .then((turns) => {
          if (!cancelled) setOpenTurns(turns);
        })
        .catch(() => {
          if (!cancelled) setOpenTurns([]);
        });
    };
    refresh();
    const unsubscribe = subscribeAgentTurns(refresh);
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, [threadId]);

  const groups = useMemo(() => {
    const raw = mode === "backlog" ? buildBacklogGroups(backlog) : buildGroups(threadWork);
    return onlyStates || excludeStates ? applyStateFilter(raw, { only: onlyStates, exclude: excludeStates }) : raw;
  }, [mode, threadWork, backlog, onlyStates, excludeStates]);

  // Flat top-to-bottom list of refs in the order they appear on screen.
  // Rebuilt whenever the groups change so ↑/↓ navigation stays in sync
  // with the section split in TaskGroupList (In progress → Ready →
  // Blocked → Done); within a section, list order (a stable sort).
  const navigableIds = useMemo(() => {
    const ids: string[] = [];
    for (const group of groups) {
      const sorted = group.items.slice().sort((a, b) => statusOrderRank(a.state) - statusOrderRank(b.state));
      for (const item of sorted) {
        ids.push(item.ref);
        const children = group.epicChildren.get(item.ref);
        if (children) {
          for (const child of children) ids.push(child.ref);
        }
      }
    }
    return ids;
  }, [groups]);

  useEffect(() => {
    if (!selectedId) return;
    if (!navigableIds.includes(selectedId)) setSelectedId(null);
  }, [navigableIds, selectedId]);

  // Prune any marked ids that no longer exist in the visible list — keeps the
  // mark set from accumulating stale entries after a move/delete/status change
  // pulls a row out from under the user.
  useEffect(() => {
    setMarkedIds((prev) => {
      if (prev.size === 0) return prev;
      const live = new Set(navigableIds);
      let changed = false;
      const next = new Set<string>();
      for (const id of prev) {
        if (live.has(id)) next.add(id);
        else changed = true;
      }
      return changed ? next : prev;
    });
  }, [navigableIds]);

  const handleSelect = (id: string, modifiers?: { toggle?: boolean; range?: boolean }) => {
    const toggle = modifiers?.toggle ?? false;
    const range = modifiers?.range ?? false;
    if (toggle) {
      // Cmd/Ctrl+click: flip the row in/out of the mark set without changing
      // selectedId (so the kb-focused row and the expand state stay put).
      setMarkedIds((prev) => {
        const next = new Set(prev);
        if (next.has(id)) next.delete(id);
        else next.add(id);
        return next;
      });
      return;
    }
    if (range && selectedId && selectedId !== id) {
      // Shift+click: mark every row between selectedId and id (inclusive of
      // both endpoints) in screen order. Selected anchor itself stays the
      // primary.
      const fromIdx = navigableIds.indexOf(selectedId);
      const toIdx = navigableIds.indexOf(id);
      if (fromIdx >= 0 && toIdx >= 0) {
        const [lo, hi] = fromIdx <= toIdx ? [fromIdx, toIdx] : [toIdx, fromIdx];
        const next = new Set<string>();
        for (let i = lo; i <= hi; i++) next.add(navigableIds[i]!);
        setMarkedIds(next);
        return;
      }
    }
    // Plain click: clear marks and move the primary selection.
    setMarkedIds(new Set());
    setSelectedId(id);
  };


  const selectedItem: WorkItem | null = useMemo(() => {
    if (!selectedId) return null;
    for (const group of groups) {
      const hit = group.items.find((item) => item.ref === selectedId);
      if (hit) return hit;
      for (const children of group.epicChildren.values()) {
        const childHit = children.find((item) => item.ref === selectedId);
        if (childHit) return childHit;
      }
    }
    return null;
  }, [groups, selectedId]);

  const activeUpdate = onUpdateTask;
  const activeDelete = features.delete ? onDeleteTask : null;
  const activeReorder = features.ordering ? (mode === "backlog" ? onReorderBacklog : onReorderTasks) : null;
  const currentScopeThreadId = mode === "backlog" ? null : thread?.id ?? null;

  useEffect(() => {
    // Listen at the pane level (not window) so the Agent pane / editor don't
    // steal the shortcut when they're focused, AND so the Plan pane can
    // keep a visible "selected" row without grabbing focus away from the
    // rest of the app. We still honour editable-target suppression for
    // typing comfort.
    const el = paneRef.current;
    if (!el) return;
    const allItems = groups.flatMap((g) => [
      ...g.items,
      ...[...g.epicChildren.values()].flat(),
    ]);
    const handler = (event: KeyboardEvent) => {
      if (kbPicker) return; // modal owns keyboard
      if (isEditableTarget(event.target)) return;
      const key = event.key;
      if ((key === "ArrowDown" || key === "ArrowUp") && event.shiftKey) {
        // Shift+↑/↓ reorders the selected item within its own status
        // section. Crossing a section boundary is a no-op — for that,
        // the user drags, which intentionally changes status as a side
        // effect. Reordering is section-local so the keyboard path
        // doesn't silently promote/demote.
        if (!selectedId || !activeReorder) return;
        const selected = allItems.find((item) => item.ref === selectedId);
        if (!selected) return;
        const selSection = classifyState(selected.state);
        const sectionIds = navigableIds.filter((id) => {
          const item = allItems.find((i) => i.ref === id);
          return item ? classifyState(item.state) === selSection : false;
        });
        const posInSection = sectionIds.indexOf(selectedId);
        const neighborPosInSection = key === "ArrowDown" ? posInSection + 1 : posInSection - 1;
        if (neighborPosInSection < 0 || neighborPosInSection >= sectionIds.length) return;
        event.preventDefault();
        const neighborId = sectionIds[neighborPosInSection]!;
        const nextOrder = navigableIds.slice();
        const i = nextOrder.indexOf(selectedId);
        const j = nextOrder.indexOf(neighborId);
        if (i < 0 || j < 0) return;
        [nextOrder[i], nextOrder[j]] = [nextOrder[j]!, nextOrder[i]!];
        void runWithError("Reorder items", activeReorder(nextOrder));
        return;
      }
      if (key === "ArrowDown" || key === "ArrowUp") {
        if (navigableIds.length === 0) return;
        event.preventDefault();
        const idx = selectedId ? navigableIds.indexOf(selectedId) : -1;
        const next = key === "ArrowDown"
          ? Math.min(idx + 1, navigableIds.length - 1)
          : idx <= 0 ? 0 : idx - 1;
        setSelectedId(navigableIds[next] ?? null);
      } else if (key === "Enter" && selectedId) {
        event.preventDefault();
        const item = allItems.find((i) => i.ref === selectedId);
        if (item) openEditModal(item);
      } else if ((key === "s" || key === "S") && selectedId) {
        if (allItems.find((i) => i.ref === selectedId)?.state === "in_progress") return;
        event.preventDefault();
        setKbPicker({ kind: "state", itemId: selectedId });
      } else if ((key === "p" || key === "P") && selectedId && editableEnums[0]) {
        // P: the list's first editable enum field (oxplow's: priority).
        event.preventDefault();
        setKbPicker({ kind: "field", field: editableEnums[0], itemId: selectedId });
      }
    };
    el.addEventListener("keydown", handler);
    return () => el.removeEventListener("keydown", handler);
  }, [navigableIds, selectedId, kbPicker, groups, activeReorder, editableEnums]);

  const openCreateModal = (parentRef: string | null = null) => {
    // Creation always routes through a full-tab NewTaskPage. Tests /
    // standalone harnesses must wire `onOpenNewTaskPage`.
    onOpenNewTaskPage?.({ parentRef });
  };

  // Register the imperative opener with the parent so menu-click
  // dispatches can open the modal without going through setState +
  // useEffect. React 18 only flushes effects synchronously for discrete
  // user input events — IPC messages from the main process aren't
  // discrete, so the openNewRequest useEffect below would stall on the
  // scheduler until the next real input event. The direct call path
  // lets setModalMode commit inside App's flushSync wrap.
  useEffect(() => {
    if (!registerOpenCreate) return;
    registerOpenCreate(() => openCreateModal());
    return () => registerOpenCreate(() => {});
    // openCreateModal captures stable setState refs, so omitting it
    // from deps is intentional.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [registerOpenCreate]);

  const openEditModal = (item: WorkItem) => {
    // Row clicks (and Enter on the keyboard-selected row) open the item's
    // page, which renders a readable view with inline editing.
    setSelectedId(item.ref);
    onOpenTaskPage?.(item.ref);
  };

  useEffect(() => {
    if (openNewRequest === undefined || openNewRequest === 0) return;
    openCreateModal();
    // openCreateModal is intentionally not in deps — it closes over setters
    // that are stable across renders.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openNewRequest]);

  useEffect(() => {
    if (!editRequest) return;
    const allItems = groups.flatMap((g) => [
      ...g.items,
      ...g.items.flatMap((item) => g.epicChildren?.get(item.ref) ?? []),
    ]);
    const item = allItems.find((i) => i.ref === editRequest.ref);
    if (item) openEditModal(item);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editRequest?.token]);


  if (mode === "thread" && !thread) {
    return <div style={{ padding: 12, color: "var(--muted)" }}>No thread selected.</div>;
  }

  const handleBacklogChipDragOver = (event: React.DragEvent) => {
    if (!dragHasWorkItems(event)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    if (!backlogChipDragOver) setBacklogChipDragOver(true);
  };

  const handleBacklogChipDrop = (event: React.DragEvent) => {
    const drag = decodeWorkItemDrag(event.dataTransfer.getData(WORK_ITEM_DRAG_MIME));
    setBacklogChipDragOver(false);
    if (!drag) return;
    event.preventDefault();
    const fromThreadId = drag.fromThreadId;
    if (!fromThreadId) return;
    // Move each carried item in sequence — one at a time keeps the failure
    // mode simple.
    for (const ref of drag.refs) void onMoveItemToBacklog(ref, fromThreadId);
  };

  return (
    <div
      ref={paneRef}
      tabIndex={0}
      data-testid="plan-pane"
      onClick={() => paneRef.current?.focus()}
      style={{ display: "flex", flexDirection: "column", height: "100%", overflow: "hidden", outline: "none" }}
    >
      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {mode === "thread" && threadId ? (
          <LensSlots
            slot="thread.plan.header"
            params={numericRowId(threadId) === null ? null : { thread_id: numericRowId(threadId) }}
            streamId={streamId}
            variant="strip"
          />
        ) : null}
        {(() => {
          const allItems = groups.flatMap((g) => [
            ...g.items,
            ...[...g.epicChildren.values()].flat(),
          ]);
          const markedItems = [...markedIds]
            .map((id) => allItems.find((item) => item.ref === id))
            .filter((item): item is WorkItem => item !== undefined);
          if (markedItems.length === 0) return null;
          return (
            <SelectionActionBar
              items={markedItems}
              fields={fields}
              onClear={() => setMarkedIds(new Set())}
              onChangeStatus={() => {
                const liveIds = markedItems
                  .filter((item) => item.state !== "in_progress")
                  .map((item) => item.ref);
                if (liveIds.length === 0) return;
                const anchor = markedItems.find((item) => item.state !== "in_progress") ?? markedItems[0]!;
                setSelectedId(anchor.ref);
                setKbPicker({ kind: "state", itemId: anchor.ref, extraIds: liveIds.filter((id) => id !== anchor.ref) });
              }}
              onChangeField={(field) => {
                const anchor = markedItems[0]!;
                setSelectedId(anchor.ref);
                setKbPicker({
                  kind: "field",
                  field,
                  itemId: anchor.ref,
                  extraIds: markedItems.map((item) => item.ref).filter((id) => id !== anchor.ref),
                });
              }}
              onAddAllToAgent={() => {
                // The same mention the "Add to agent context" menu item
                // makes, chained.
                insertIntoAgent(markedItems.map(mention).join(""));
              }}
              onDelete={activeDelete ? () => {
                const liveIds = markedItems
                  .filter((item) => item.state !== "in_progress")
                  .map((item) => item.ref);
                if (liveIds.length === 0) return;
                for (const id of liveIds) void activeDelete(id);
                showToast({ message: `Deleted ${liveIds.length} item${liveIds.length === 1 ? "" : "s"}.` });
                setMarkedIds(new Set());
              } : undefined}
            />
          );
        })()}
        {groups.length === 0 ? (
          <>
            <div style={{ padding: 12, color: "var(--muted)", fontSize: "var(--text-xs)" }}>
              Nothing here.
            </div>
          </>
        ) : (
          groups.map((group) => {
            const isRootThread = mode === "thread";
            const isActive = isRootThread && thread?.id === activeThreadId;
            const readyMenuItems: MenuItem[] = [
              {
                id: "plan-new-task",
                label: "New item",
                shortcut: "⇧⌘N",
                enabled: true,
                run: () => openCreateModal(),
              },
            ];
            const readyActions = (
              <span data-testid="plan-add-points-bar">
                <SectionHeaderMenu items={readyMenuItems} testId="plan-ready-menu" />
              </span>
            );
            const sectionActions: Partial<Record<TaskSectionKind, React.ReactNode>> = {
              ready: readyActions,
            };
            if (extraSectionLinks) {
              for (const [k, node] of Object.entries(extraSectionLinks) as Array<[TaskSectionKind, React.ReactNode]>) {
                if (!node) continue;
                const existing = sectionActions[k];
                sectionActions[k] = existing ? (
                  <>{existing}{node}</>
                ) : node;
              }
            }
            return (
              <TaskGroupList
                key={group.epic?.ref ?? "__root__"}
                group={group}
                scopeThreadId={currentScopeThreadId}
                fields={fields}
                onUpdateTask={activeUpdate}
                onReorderTasks={activeReorder ?? undefined}
                onOpenMenu={(rect, item) => {
                  const groupIds = markedIds.has(item.ref) && markedIds.size > 1
                    ? [...markedIds]
                    : null;
                  setContextMenu({ x: rect.right, y: rect.bottom + 4, item, groupIds });
                }}
                sectionActions={sectionActions}
                selectedId={selectedId}
                markedIds={markedIds}
                onSelect={handleSelect}
                onRequestEdit={openEditModal}
                epicChildrenMap={group.epicChildren}
                onReparentTask={features.hierarchy ? (ref, parentRef) => activeUpdate(ref, { parentRef }) : undefined}
                onAddChildTask={features.hierarchy ? (epicRef) => openCreateModal(epicRef) : undefined}
                isActive={isActive}
                agentStatus={agentStatus}
                isSectionCollapsed={isSectionCollapsed}
                onToggleSectionCollapsed={onToggleSectionCollapsed}
                openTurns={mode === "thread" && !group.epic ? openTurns : []}
                followups={isRootThread && !group.epic ? threadWork?.followups ?? [] : []}
                onDismissFollowup={isRootThread && threadId
                  ? (id) => runWithError("Dismiss follow-up", removeFollowup(threadId, id))
                  : undefined}
                visibleSections={visibleSections}
                sectionItemLimit={sectionItemLimit}
              />
            );
          })
        )}
      </div>
      {hideBacklogChip || forceMode || !features.lists ? null : (
      <div style={bottomBarStyle}>
        <button type="button"
          onClick={() => setInternalMode((prev) => (prev === "backlog" ? "thread" : "backlog"))}
          onDragOver={handleBacklogChipDragOver}
          onDragLeave={() => setBacklogChipDragOver(false)}
          onDrop={handleBacklogChipDrop}
          style={{
            ...bottomChipStyle,
            background: mode === "backlog" ? "var(--accent)" : "var(--bg-2)",
            color: mode === "backlog" ? "#fff" : "inherit",
            borderColor: backlogChipDragOver ? "var(--accent)" : "var(--border)",
            boxShadow: backlogChipDragOver ? "0 0 0 2px var(--accent)" : undefined,
          }}
          title="Backlog (global across streams)"
        >
          Backlog{backlog ? ` · ${backlog.items.length}` : ""}
        </button>
      </div>
      )}
      {contextMenu ? (
        <ContextMenu
          items={contextMenu.groupIds
            ? buildGroupMenu(contextMenu.item, contextMenu.groupIds, editableEnums, {
                onChangeStatus: (item, ids) => {
                  setContextMenu(null);
                  setSelectedId(item.ref);
                  const allWi = groups.flatMap((g) => [...g.items, ...[...g.epicChildren.values()].flat()]);
                  const liveIds = ids.filter((id) => allWi.find((i) => i.ref === id)?.state !== "in_progress");
                  setKbPicker({ kind: "state", itemId: item.ref, extraIds: liveIds.filter((id) => id !== item.ref) });
                },
                onChangeField: (item, ids, field) => {
                  setContextMenu(null);
                  setSelectedId(item.ref);
                  setKbPicker({ kind: "field", field, itemId: item.ref, extraIds: ids.filter((id) => id !== item.ref) });
                },
                onDelete: activeDelete ? (_item, ids) => {
                  setContextMenu(null);
                  const allWi = groups.flatMap((g) => [...g.items, ...[...g.epicChildren.values()].flat()]);
                  const liveIds = ids.filter((id) => allWi.find((i) => i.ref === id)?.state !== "in_progress");
                  if (liveIds.length === 0) return;
                  for (const id of liveIds) void activeDelete(id);
                  showToast({ message: `Deleted ${liveIds.length} item${liveIds.length === 1 ? "" : "s"}.` });
                } : null,
                onAddToAgent: (ids) => {
                  setContextMenu(null);
                  const allWi = groups.flatMap((g) => [...g.items, ...[...g.epicChildren.values()].flat()]);
                  const text = ids
                    .map((id) => allWi.find((i) => i.ref === id))
                    .filter((item): item is WorkItem => item !== undefined)
                    .map(mention)
                    .join("");
                  if (text.length > 0) insertIntoAgent(text);
                },
              })
            : buildItemMenu(contextMenu.item, editableEnums, {
                onDelete: activeDelete ? (item) => {
                  setContextMenu(null);
                  if (expandedId === item.ref) setExpandedId(null);
                  void activeDelete(item.ref);
                  showToast({ message: `Deleted "${item.title}".` });
                } : null,
                onRename: (item) => {
                  setContextMenu(null);
                  setSelectedId(item.ref);
                  openEditModal(item);
                },
                onChangeStatus: (item) => {
                  setContextMenu(null);
                  setSelectedId(item.ref);
                  setKbPicker({ kind: "state", itemId: item.ref });
                },
                onChangeField: (item, field) => {
                  setContextMenu(null);
                  setSelectedId(item.ref);
                  setKbPicker({ kind: "field", field, itemId: item.ref });
                },
                onAddToAgent: (item) => {
                  setContextMenu(null);
                  insertIntoAgent(mention(item));
                },
                onComment: (item) => {
                  setContextMenu(null);
                  openCommentForTask(item);
                },
              })}
          position={{ x: contextMenu.x, y: contextMenu.y }}
          onClose={() => setContextMenu(null)}
          minWidth={160}
        />
      ) : null}
      {kbPicker && selectedItem ? (
        <KeyboardValuePicker
          picker={kbPicker}
          item={selectedItem}
          onPick={(value) => {
            const allIds = kbPicker.extraIds
              ? [kbPicker.itemId, ...kbPicker.extraIds]
              : [kbPicker.itemId];
            if (kbPicker.kind === "state") {
              for (const id of allIds) void activeUpdate(id, { state: value as CanonicalState });
            } else {
              const name = kbPicker.field.name;
              for (const id of allIds) void activeUpdate(id, { native: { [name]: value } });
            }
            setKbPicker(null);
            paneRef.current?.focus();
          }}
          onClose={() => { setKbPicker(null); paneRef.current?.focus(); }}
        />
      ) : null}
    </div>
  );
}

/**
 * Small centered picker opened by the keyboard shortcuts `S` (state) /
 * `P` (the list's first enum field) when a row is selected. Autofocuses, ↑/↓ navigate options, Enter
 * commits, Escape cancels. Mouse click on a row also commits. Kept in-line
 * in this file rather than extracted because nothing else uses it.
 */
function KeyboardValuePicker({
  picker,
  item,
  onPick,
  onClose,
}: {
  picker: KbPickerKind;
  item: WorkItem;
  onPick(value: string): void;
  onClose(): void;
}) {
  const options: readonly string[] = picker.kind === "state" ? CANONICAL_STATES : picker.field.values;
  const current = picker.kind === "state" ? item.state : String(item.native[picker.field.name] ?? "");
  const initialIdx = Math.max(0, options.indexOf(current as string));
  const [idx, setIdx] = useState(initialIdx);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setIdx((prev) => Math.min(prev + 1, options.length - 1));
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        setIdx((prev) => Math.max(prev - 1, 0));
      } else if (event.key === "Enter") {
        event.preventDefault();
        onPick(options[idx]!);
      } else if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [idx, options, onPick, onClose]);

  return (
    <div style={kbPickerOverlayStyle} onClick={onClose}>
      <div style={kbPickerStyle} onClick={(event) => event.stopPropagation()}>
        <div style={{ padding: "8px 12px", borderBottom: "1px solid var(--border)", fontSize: 11, color: "var(--muted)", textTransform: "uppercase", letterSpacing: 0.6 }}>
          {picker.kind === "state" ? "Set state" : `Set ${picker.field.title.toLowerCase()}`}
          <span style={{ float: "right", fontFamily: "ui-monospace, monospace" }}>↑↓ · Enter · Esc</span>
        </div>
        <div style={{ padding: 4 }}>
          {options.map((option, i) => {
            const active = i === idx;
            return (
              <div
                key={option}
                onMouseEnter={() => setIdx(i)}
                onClick={() => onPick(option)}
                style={{
                  padding: "5px 10px",
                  borderRadius: 4,
                  fontSize: "var(--text-sm)",
                  cursor: "pointer",
                  background: active ? "var(--accent)" : "transparent",
                  color: active ? "#fff" : "var(--fg)",
                }}
              >
                {picker.kind === "state" ? statusLabel(option as CanonicalState) : option.replace(/_/g, " ")}
                {option === current ? <span style={{ marginLeft: 8, opacity: 0.7 }}>· current</span> : null}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

const kbPickerOverlayStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.4)",
  display: "flex",
  alignItems: "flex-start",
  justifyContent: "center",
  paddingTop: "20vh",
  zIndex: 3000,
};

const kbPickerStyle: CSSProperties = {
  background: "var(--bg-1)",
  border: "1px solid var(--border-strong)",
  borderRadius: 8,
  width: "min(280px, 90vw)",
  boxShadow: "0 12px 32px rgba(0,0,0,0.6)",
};


/// Open the comment composer anchored to an item row's title. Used by
/// the row context menu's "Comment…" item — rows are draggable, so a
/// drag-select can't start on them; this is the right-click path. Finds
/// the row's `data-ref` element and dispatches a pending comment to the
/// app-level layer via the compose bus.
function openCommentForTask(item: WorkItem): void {
  const el = document.querySelector(
    `[data-ref-kind="work_item"][data-ref-id="${CSS.escape(item.ref.replace(/^work_item:/, ""))}"]`,
  );
  if (!el) return;
  const req = composeForElement(el, item.title, el.getBoundingClientRect());
  if (req) requestCommentCompose(req);
}

function buildItemMenu(
  item: WorkItem,
  fields: FieldDecl[],
  actions: {
    onDelete: ((item: WorkItem) => void) | null;
    onRename: (item: WorkItem) => void;
    onChangeStatus: (item: WorkItem) => void;
    onChangeField: (item: WorkItem, field: FieldDecl) => void;
    onAddToAgent: (item: WorkItem) => void;
    onComment: (item: WorkItem) => void;
  },
): MenuItem[] {
  const locked = item.state === "in_progress";
  const items: MenuItem[] = [
    { id: "tasks.rename", label: "Rename…", enabled: !locked, run: () => actions.onRename(item) },
    { id: "tasks.comment", label: "Comment…", enabled: true, run: () => actions.onComment(item) },
    { id: "tasks.status", label: "Change state…", enabled: !locked, run: () => actions.onChangeStatus(item) },
    ...fields.map((field) => ({
      id: `tasks.field.${field.name}`,
      label: `Change ${field.title.toLowerCase()}…`,
      enabled: true,
      run: () => actions.onChangeField(item, field),
    })),
    { id: "tasks.add-to-agent", label: "Add to agent context", enabled: true, run: () => actions.onAddToAgent(item) },
  ];
  const onDelete = actions.onDelete;
  if (onDelete) items.push({ id: "tasks.delete", label: "Delete", enabled: !locked, run: () => onDelete(item) });
  return items;
}

function buildGroupMenu(
  item: WorkItem,
  groupIds: string[],
  fields: FieldDecl[],
  actions: {
    onChangeStatus: (item: WorkItem, ids: string[]) => void;
    onChangeField: (item: WorkItem, ids: string[], field: FieldDecl) => void;
    onDelete: ((item: WorkItem, ids: string[]) => void) | null;
    onAddToAgent: (ids: string[]) => void;
  },
): MenuItem[] {
  const locked = item.state === "in_progress";
  const n = groupIds.length;
  const items: MenuItem[] = [
    { id: "tasks.status", label: `Change state… (${n} items)`, enabled: !locked, run: () => actions.onChangeStatus(item, groupIds) },
    ...fields.map((field) => ({
      id: `tasks.field.${field.name}`,
      label: `Change ${field.title.toLowerCase()}… (${n} items)`,
      enabled: true,
      run: () => actions.onChangeField(item, groupIds, field),
    })),
    { id: "tasks.add-to-agent", label: `Add to agent context (${n} items)`, enabled: true, run: () => actions.onAddToAgent(groupIds) },
  ];
  const onDelete = actions.onDelete;
  if (onDelete) items.push({ id: "tasks.delete", label: `Delete (${n} items)`, enabled: !locked, run: () => onDelete(item, groupIds) });
  return items;
}

const bottomBarStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  padding: "6px 8px",
  borderTop: "1px solid var(--border)",
  background: "var(--bg)",
};

const bottomChipStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 6,
  padding: "3px 10px",
  border: "1px solid var(--border)",
  borderRadius: 999,
  background: "var(--bg-2)",
  color: "inherit",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 11,
  whiteSpace: "nowrap",
};

