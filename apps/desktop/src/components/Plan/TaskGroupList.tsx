import type { CSSProperties } from "react";
import { Fragment, useEffect, useMemo, useState } from "react";
import type { AgentStatus, OpenAgentTurn, ThreadFollowup } from "../../api.js";
import { decodeWorkItemDrag, dragHasWorkItems, setWorkItemDrag } from "../../agent-context-dnd.js";
import { WORK_ITEM_DRAG_MIME } from "../../dragMimes.js";
import { CANONICAL_STATES, type CanonicalState, type FieldDecl, type WorkItem } from "../../workItems.js";
import { FieldBadge, fieldText } from "../WorkItemFields.js";
import {
  classifyRow,
  finalizeReorderRefs,
  sectionDefaultState,
  miniButtonStyle,
  sectionActionButtonStyle,
  sectionHeaderStyle,
  statusIcon,
  statusLabel,
  type PlanSectionKey,
  type TaskGroup,
  type TaskSectionKind,
} from "./plan-utils.js";
import type { TaskDetailChanges } from "./TaskDetail.js";
import { ContextMenu } from "../ContextMenu.js";
import type { MenuItem } from "../../menu.js";

/**
 * Renders one group of a work list (an epic + its children, or the root
 * group with no epic). Items are split by state into four sections —
 * In progress → Ready → Blocked → Done — with dividers between non-empty
 * sections.
 *
 * Drag-reorder places the dragged item on its list (`oxplow.work_item.reorder`,
 * offered only with the list's `ordering`). Dragging an item across
 * section boundaries also moves it to that section's state (ready → todo,
 * done → done) so the person can triage straight from the list.
 * InProgress rejects drop-in: the agent owns that state and in-progress
 * items are drag-locked. Empty sections stay hidden until a drag is
 * active, at which point they appear as drop targets.
 */
export type QueueRow = { kind: "work"; id: string; item: WorkItem };

interface SectionBucket {
  kind: TaskSectionKind;
  label: string;
  rows: QueueRow[];
}

const SECTION_ORDER: Array<{ kind: TaskSectionKind; label: string }> = [
  { kind: "inProgress", label: "In progress" },
  { kind: "ready", label: "Ready" },
  { kind: "blocked", label: "Blocked" },
  { kind: "done", label: "Done" },
];

export function TaskGroupList({
  group,
  scopeThreadId,
  onUpdateTask,
  onReorderTasks,
  onOpenMenu,
  sectionActions,
  selectedId,
  markedIds,
  onSelect,
  onRequestEdit,
  epicChildrenMap,
  onReparentTask,
  onAddChildTask,
  isActive,
  agentStatus,
  isSectionCollapsed,
  onToggleSectionCollapsed,
  openTurns,
  followups,
  onDismissFollowup,
  visibleSections,
  sectionItemLimit,
  fields,
}: {
  group: TaskGroup;
  scopeThreadId: string | null;
  onUpdateTask: (ref: string, changes: TaskDetailChanges) => Promise<void>;
  /** Place a dragged item on its list; absent when the list keeps no
   *  order of its own (drags then only restate). */
  onReorderTasks?: (orderedRefs: string[]) => Promise<void>;
  onOpenMenu(rect: DOMRect, item: WorkItem): void;
  /** Per-section action buttons (right-aligned in each section header).
   *  The PlanPane builds this map and threads it in — add new per-section
   *  commands here rather than in the header rendering. Done's built-in
   *  archive controls render alongside whatever's passed for `done`. */
  sectionActions?: Partial<Record<TaskSectionKind, React.ReactNode>>;
  selectedId?: string | null;
  markedIds?: ReadonlySet<string>;
  onSelect?(id: string, modifiers?: { toggle?: boolean; range?: boolean }): void;
  onRequestEdit?(item: WorkItem): void;
  epicChildrenMap: Map<string, WorkItem[]>;
  /** Move an item under an epic (or out of one, `null`); absent when the
   *  list doesn't nest. */
  onReparentTask?: (ref: string, newParentRef: string | null) => Promise<void>;
  onAddChildTask?: (epicRef: string) => void;
  isActive?: boolean;
  /** Live agent state for this thread, used to drive the In Progress
   *  empty-state placeholder ("Thinking..." with a braille spinner when
   *  the agent is `working`, "Waiting" when `waiting`). Falls back to
   *  `waiting` when undefined. */
  agentStatus?: AgentStatus;
  /** Collapse-state accessors from PlanPane's useCollapsedSections. */
  isSectionCollapsed: (kind: PlanSectionKey) => boolean;
  onToggleSectionCollapsed: (kind: PlanSectionKey) => void;
  /** Open agent turns (`ended_at IS NULL`) for this thread. Each
   *  renders as a non-interactive live row — spinner + prompt — at the
   *  top of the In Progress section, and disappears when the Stop hook
   *  closes the turn. Observational only: no status, no drag, no menu.
   *  Only the root group renders them. */
  openTurns?: OpenAgentTurn[];
  /** Transient agent follow-ups for this thread (in-memory on the
   *  runtime, lost on restart). Rendered at the very top of the Ready
   *  section as italic muted "↳ follow-up: …" lines with a single ✕
   *  dismiss button. Only the root group renders them — children of
   *  epics inherit nothing. */
  followups?: ThreadFollowup[];
  onDismissFollowup?: (id: string) => void;
  /** When provided, only sections in this list render. Used by the
   *  page split (Plan Work / Done Work / Archived) to restrict the
   *  panel to a subset of the five buckets. Default = all. */
  visibleSections?: TaskSectionKind[];
  /** Cap rows per section. Used by Plan Work to render previews of
   *  Done. Sections with no entry render fully. */
  sectionItemLimit?: Partial<Record<TaskSectionKind, number>>;
  /** The list's own fields: its editable enums show (and change) on
   *  each row. */
  fields: FieldDecl[];
}) {
  // When the thread is not the active writer, in_progress items are not agent-owned
  // and can be freely reordered — only lock them when this thread is active.
  const lockInProgress = isActive !== false;
  const [draggingKey, setDraggingKey] = useState<string | null>(null);
  const [overKey, setOverKey] = useState<string | null>(null);
  const [overSection, setOverSection] = useState<TaskSectionKind | null>(null);
  const [expandedEpicIds, setExpandedEpicIds] = useState<Set<string>>(() => new Set());

  const { sections, allRows } = useMemo(() => {
    const work: QueueRow[] = group.items.map((item) => ({ kind: "work" as const, id: item.ref, item }));
    const buckets: Record<TaskSectionKind, QueueRow[]> = {
      inProgress: [], ready: [], blocked: [], done: [],
    };
    for (const row of work) {
      // Epics roll up their children's statuses into an effective section
      // (`classifyEpic`) so the epic + its children render as one block in
      // whichever section the rollup picks. Non-epics use their literal
      // status. See plan-utils.ts.
      buckets[classifyRow(row.item, epicChildrenMap)].push(row);
    }
    const orderedSections: SectionBucket[] = [];
    const flat: QueueRow[] = [];
    const allowedKinds = visibleSections ? new Set(visibleSections) : null;
    for (const { kind, label } of SECTION_ORDER) {
      if (allowedKinds && !allowedKinds.has(kind)) continue;
      // Rows arrive in list order. Done renders descending — the latest
      // on the list on top, so the person can triage (or reopen) them
      // without scrolling. `finalizeReorderRefs` unwinds descending runs
      // when a reorder is sent, so the list keeps one order.
      if (kind === "done") buckets[kind].reverse();
      // Keep empty sections in the list while a drag is active so the user
      // can drop into an empty "Done" to create the first item there.
      // When nothing is dragging, empty sections are suppressed by the
      // renderer below.
      const limit = sectionItemLimit?.[kind];
      const limited = typeof limit === "number" ? buckets[kind].slice(0, limit) : buckets[kind];
      if (limited.length === 0) {
        orderedSections.push({ kind, label, rows: [] });
      } else {
        orderedSections.push({ kind, label, rows: limited });
        // The DnD index (`allRows`) only needs the visible rows — items
        // hidden by the preview cap aren't drag targets on this surface.
        flat.push(...limited);
      }
    }
    return { sections: orderedSections, allRows: flat };
  }, [group.items, epicChildrenMap, visibleSections, sectionItemLimit]);

  // Index every item visible in this group (root + every epic's
  // children) so the drag-start handler can encode each carried item's
  // {ref, title, state} into the work-item drag. The agent terminal reads
  // it to add each marked row as a context ref without its own lookup.
  const allItemsByRef = useMemo(() => {
    const map = new Map<string, WorkItem>();
    for (const item of group.items) map.set(item.ref, item);
    for (const children of epicChildrenMap.values()) {
      for (const child of children) map.set(child.ref, child);
    }
    return map;
  }, [group.items, epicChildrenMap]);

  const keyFor = (row: { kind: string; id: string }) => `${row.kind}:${row.id}`;

  // Look up the dragged tasks (if any) so cross-section drops can route
  // through onUpdateTask. Commit/wait rows don't have a status to change.
  const draggedTask = (() => {
    if (!draggingKey) return null;
    const row = allRows.find((r) => keyFor(r) === draggingKey);
    return row && row.kind === "work" ? row.item : null;
  })();

  const resetDrag = () => { setDraggingKey(null); setOverKey(null); setOverSection(null); };

  const handleDropOnKey = (targetKey: string) => {
    if (!draggingKey || draggingKey === targetKey) { resetDrag(); return; }
    const from = allRows.findIndex((row) => keyFor(row) === draggingKey);
    const to = allRows.findIndex((row) => keyFor(row) === targetKey);
    if (from < 0 || to < 0) { resetDrag(); return; }
    const dragged = allRows[from]!;
    const target = allRows[to]!;
    // Manual reorder *within* the Done section is intentionally a
    // no-op: Done renders the latest first. Cross-section drops
    // (Done ↔ another section) still flow through to state changes
    // below.
    if (
      dragged.kind === "work" && target.kind === "work" &&
      classifyRow(dragged.item, epicChildrenMap) === "done" &&
      classifyRow(target.item, epicChildrenMap) === "done"
    ) {
      resetDrag();
      return;
    }
    const isMultiDrag = dragged.kind === "work" && markedIds && markedIds.has(dragged.item.ref) && markedIds.size > 1;
    // Track any state changes the drop implies so we can feed the *effective*
    // new state into finalizeReorderRefs below — otherwise the dragged row
    // would still look like its old section to the run detector, which
    // miscomputes the descending-run flips (regression when dragging out of
    // Done back to Ready).
    const statusOverrides = new Map<string, CanonicalState>();
    // Cross-section drop — change status to match the target section.
    // When it's a multi-drag, apply the status change to every marked item.
    if (dragged.kind === "work" && target.kind === "work") {
      const fromSection = classifyRow(dragged.item, epicChildrenMap);
      const toSection = classifyRow(target.item, epicChildrenMap);
      // Epics never carry a literal status of their own — their section
      // is computed from children. Don't try to mutate an epic's status
      // when it crosses a section boundary; the rollup will follow once
      // its children change.
      if (fromSection !== toSection && !(group.epicChildren.get(dragged.item.ref) ?? []).length) {
        const nextStatus = sectionDefaultState(toSection);
        if (nextStatus) {
          if (isMultiDrag && markedIds) {
            for (const id of markedIds) {
              const row = allRows.find((r) => r.kind === "work" && r.id === id);
              if (row && row.kind === "work" && !(group.epicChildren.get(row.item.ref) ?? []).length && row.item.state !== nextStatus) {
                void onUpdateTask(id, { state: nextStatus });
                statusOverrides.set(id, nextStatus);
              }
            }
          } else if (nextStatus !== dragged.item.state) {
            void onUpdateTask(dragged.item.ref, { state: nextStatus });
            statusOverrides.set(dragged.item.ref, nextStatus);
          }
        }
      }
    }
    // Determine whether this drop lands in the Done section. Done has a
    // "drop-to-top" contract: dropped items land at its top rather than
    // wherever the pointer hit — the head of the Done run in visual order
    // (Done renders descending).
    const targetSection = target.kind === "work" ? classifyRow(target.item, epicChildrenMap) : null;
    const dropsIntoDone = targetSection === "done";

    // Reorder: multi-drag moves all marked rows as a block to the drop position.
    let next: QueueRow[];
    if (isMultiDrag && markedIds) {
      const markedSet = new Set(markedIds);
      const markedRows = allRows.filter((r) => r.kind === "work" && markedSet.has(r.id));
      const unmarked = allRows.filter((r) => r.kind !== "work" || !markedSet.has(r.id));
      let insertAt: number;
      if (dropsIntoDone) {
        // Insert at the first Done row in `unmarked` (top of Done section
        // visually, since Done renders descending). If there's no Done row
        // yet, append — this is the first Done item.
        const doneIdx = unmarked.findIndex(
          (r) => r.kind === "work" && classifyRow(r.item, epicChildrenMap) === "done",
        );
        insertAt = doneIdx < 0 ? unmarked.length : doneIdx;
      } else {
        const insertIdx = unmarked.findIndex((r) => keyFor(r) === targetKey);
        insertAt = insertIdx < 0 ? unmarked.length : insertIdx;
      }
      next = [...unmarked.slice(0, insertAt), ...markedRows, ...unmarked.slice(insertAt)];
    } else {
      next = allRows.slice();
      const [moved] = next.splice(from, 1);
      if (dropsIntoDone) {
        const doneIdx = next.findIndex(
          (r) => r.kind === "work" && classifyRow(r.item, epicChildrenMap) === "done",
        );
        const insertAt = doneIdx < 0 ? next.length : doneIdx;
        next.splice(insertAt, 0, moved!);
      } else {
        // `to` was computed before splice; if from < to the remaining index
        // after removal is `to - 1`. (Pre-existing behavior kept for the
        // non-Done path so other sections behave the same as before.)
        next.splice(to, 0, moved!);
      }
    }
    resetDrag();
    // `next` is in visual order (Done descending). Convert to list order
    // before sending — finalizeReorderRefs flips descending runs, which
    // keeps the next render's visual order stable. Use the effective
    // (post-drop) state for rows whose state just changed so the run
    // detector sees the new section membership.
    const workRowsInVisualOrder = next
      .filter((row): row is Extract<QueueRow, { kind: "work" }> => row.kind === "work")
      .map((row) => ({ ref: row.id, state: statusOverrides.get(row.id) ?? row.item.state }));
    void onReorderTasks?.(finalizeReorderRefs(workRowsInVisualOrder));
  };

  const handleDropOnSection = (section: TaskSectionKind) => {
    if (!draggedTask) { resetDrag(); return; }
    const nextStatus = sectionDefaultState(section);
    resetDrag();
    if (!nextStatus) return;
    // An epic's section is computed from its children. Section drops on
    // an epic are no-ops.
    if ((epicChildrenMap.get(draggedTask.ref) ?? []).length > 0) return;
    if (classifyRow(draggedTask, epicChildrenMap) === section) return;
    const isMultiDrag = markedIds && markedIds.has(draggedTask.ref) && markedIds.size > 1;
    if (isMultiDrag && markedIds) {
      for (const id of markedIds) {
        const row = allRows.find((r) => r.kind === "work" && r.id === id);
        if (
          row && row.kind === "work" &&
          !(group.epicChildren.get(row.item.ref) ?? []).length &&
          classifyRow(row.item, epicChildrenMap) !== section
        ) {
          void onUpdateTask(id, { state: nextStatus });
        }
      }
    } else {
      void onUpdateTask(draggedTask.ref, { state: nextStatus });
    }
  };

  const renderRow = (row: QueueRow) => {
    const key = keyFor(row);
    // Done drops always land at the top of the section, so a
    // between-rows indicator on that target would lie about where the
    // dragged item will end up. Suppress it there.
    const targetSection = classifyRow(row.item, epicChildrenMap);
    const suppressDropLine = targetSection === "done";
    const isOver = overKey === key && draggingKey !== key && !suppressDropLine;
    const isDragging = draggingKey === key;
    const isMarked = markedIds?.has(row.item.ref) ?? false;
    const sharedDragHandlers = {
      onDragStart: (event: React.DragEvent) => {
        // Populate dataTransfer FIRST and let dragstart return before
        // mutating React state that re-renders the dragged row.
        // Recent Chromium (≥ 124) treats a same-tick style/attribute
        // change on the drag source as "source detached" and silently
        // cancels the drag — the user never sees the drag image
        // animate. Deferring setDraggingKey to the next microtask
        // keeps the row's `cursor`/`background`/`isDragging` props
        // identical at the moment the browser snapshots the source,
        // so the drag actually starts. Reorder + cross-section drop
        // logic still runs as before once `dragging` state lands.
        const refs = isMarked && markedIds && markedIds.size > 1 ? [...markedIds] : [row.item.ref];
        // Each carried item's {ref, title, state}, so cross-pane drop
        // targets (e.g. the agent terminal) can build context refs without
        // their own lookup. The dragged row is always present even if not
        // in the page-local index — it's the row the person grabbed.
        const items = refs
          .map((ref) => allItemsByRef.get(ref) ?? (ref === row.item.ref ? row.item : null))
          .filter((item): item is WorkItem => item !== null)
          .map((item) => ({ ref: item.ref, title: item.title, state: item.state }));
        setWorkItemDrag(event, { refs, items, fromThreadId: scopeThreadId });
        queueMicrotask(() => setDraggingKey(key));
      },
      onDragEnd: resetDrag,
      onDragOver: (event: React.DragEvent) => {
        if (!draggingKey || draggingKey === key) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        if (overKey !== key) setOverKey(key);
      },
      onDragLeave: () => { if (overKey === key) setOverKey(null); },
      onDrop: (event: React.DragEvent) => {
        event.preventDefault();
        if (draggingKey) { handleDropOnKey(key); return; }
        // A row dragged out of an epic and dropped on the list leaves it.
        const drag = decodeWorkItemDrag(event.dataTransfer.getData(WORK_ITEM_DRAG_MIME));
        if (drag?.parentEpicRef && onReparentTask) {
          for (const ref of drag.refs) void onReparentTask(ref, null);
        }
      },
    };
    if ((group.epicChildren.get(row.item.ref) ?? []).length > 0) {
      const isExpanded = expandedEpicIds.has(row.item.ref);
      const children = epicChildrenMap.get(row.item.ref) ?? [];
      // Surface stale-epic-children: when the epic is closed but
      // children are still ready/in_progress the rollup pulls the
      // epic back into Ready, hiding the closed state. The banner
      // gives the user a one-click cascade fix.
      const epicStatus = row.item.state;
      const staleChildren =
        epicStatus === "done" || epicStatus === "blocked"
          ? children.filter((c) => c.state === "todo" || c.state === "in_progress")
          : [];
      return (
        <div key={key}>
          <EpicInlineRow
            rowKey={key}
            item={row.item}
            isExpanded={isExpanded}
            onToggleExpand={() => {
              setExpandedEpicIds((prev) => {
                const next = new Set(prev);
                if (next.has(row.item.ref)) next.delete(row.item.ref);
                else next.add(row.item.ref);
                return next;
              });
            }}
            isSelected={selectedId === row.item.ref}
            isMarked={isMarked}
            isOver={isOver}
            isDragging={isDragging}
            scopeThreadId={scopeThreadId}
            lockInProgress={lockInProgress}
            onSelect={onSelect}
            onRequestEdit={onRequestEdit}
            onUpdateTask={onUpdateTask}
            onOpenMenu={onOpenMenu}
            fields={fields}
            {...sharedDragHandlers}
          />
          {staleChildren.length > 0 ? (
            <StaleEpicChildrenBanner
              epic={row.item}
              staleChildren={staleChildren}
              onCascade={(targetStatus) => {
                for (const child of staleChildren) {
                  void onUpdateTask(child.ref, { state: targetStatus });
                }
              }}
            />
          ) : null}
          {isExpanded ? (
            <EpicChildrenPane
              epicRef={row.item.ref}
              children={children}
              onReorderTasks={onReorderTasks}
              onReparentTask={onReparentTask}
              onUpdateTask={onUpdateTask}
              onOpenMenu={onOpenMenu}
              scopeThreadId={scopeThreadId}
              onRequestEdit={onRequestEdit}
              selectedId={selectedId}
              markedIds={markedIds}
              onSelect={onSelect}
              onAddChildTask={onAddChildTask}
              fields={fields}
            />
          ) : null}
        </div>
      );
    }
    return (
      <InlineItemRow
        key={key}
        rowKey={key}
        item={row.item}
        isSelected={selectedId === row.item.ref}
        isMarked={isMarked}
        isOver={isOver}
        isDragging={isDragging}
        scopeThreadId={scopeThreadId}
        lockInProgress={lockInProgress}
        onRequestEdit={onRequestEdit}
        onSelect={onSelect}
        onUpdateTask={onUpdateTask}
        onOpenMenu={onOpenMenu}
        fields={fields}
        {...sharedDragHandlers}
      />
    );
  };

  return (
    // Sections are collapsible — the pane scrolls as a whole, and each
    // section's body only renders when expanded. Collapsed state persists
    // in localStorage so the user's layout choices stick across reloads.
    <div>
      {sections.map((section, index) => {
        const empty = section.rows.length === 0;
        const alwaysShow = section.kind === "ready" || section.kind === "done" || section.kind === "inProgress" || section.kind === "blocked";
        if (empty && !alwaysShow && !draggedTask) {
          return null;
        }
        const canDrop = !!draggedTask
          && section.kind !== "inProgress"
          && !(epicChildrenMap.get(draggedTask.ref) ?? []).length
          && classifyRow(draggedTask, epicChildrenMap) !== section.kind;
        const isOverSection = canDrop && overSection === section.kind;
        const headerDropHandlers = canDrop
          ? {
              onDragOver: (event: React.DragEvent) => {
                event.preventDefault();
                event.dataTransfer.dropEffect = "move";
                if (overSection !== section.kind) setOverSection(section.kind);
              },
              onDragLeave: () => {
                if (overSection === section.kind) setOverSection(null);
              },
              onDrop: (event: React.DragEvent) => {
                event.preventDefault();
                handleDropOnSection(section.kind);
              },
            }
          : {};
        const headerBaseStyle: CSSProperties =
          index === 0 ? firstSectionLabelStyle : sectionHeaderStyle;
        const headerStyle: CSSProperties = {
          ...headerBaseStyle,
          outline: isOverSection ? "1px solid var(--accent)" : "none",
          background: isOverSection ? "rgba(74,158,255,0.08)" : headerBaseStyle.background,
          cursor: canDrop ? "copy" : "pointer",
        };
        const renderedRows = section.rows;
        const customActions = sectionActions?.[section.kind];
        const isCollapsed = isSectionCollapsed(section.kind);
        return (
          <Fragment key={section.kind}>
          <div data-testid={`plan-section-${section.kind}`}>
            <div
              style={{ ...headerStyle, display: "flex", alignItems: "center", gap: 8 }}
              data-testid={`plan-section-header-${section.kind}`}
              onClick={() => onToggleSectionCollapsed(section.kind)}
              {...headerDropHandlers}
            >
              <span style={{ flex: 1, minWidth: 0, display: "flex", alignItems: "center", gap: 6 }}>
                <span>{section.label}</span>
              </span>
              {customActions ? (
                <span
                  onClick={(event) => event.stopPropagation()}
                  style={{ display: "flex", alignItems: "center", gap: 6, textTransform: "none", letterSpacing: 0 }}
                >
                  {customActions}
                </span>
              ) : null}
            </div>
            {!isCollapsed || empty ? (
              <>
                {!isCollapsed && section.kind === "inProgress" && openTurns && openTurns.length > 0
                  ? openTurns.map((turn) => <LiveTurnRow key={turn.id} turn={turn} />)
                  : null}
                {!isCollapsed && section.kind === "ready" && followups && followups.length > 0
                  ? followups.map((fu) => (
                      <FollowupRow
                        key={fu.id}
                        followup={fu}
                        onDismiss={onDismissFollowup}
                      />
                    ))
                  : null}
                {!isCollapsed ? renderedRows.map(renderRow) : null}
                {empty
                && !draggedTask
                && !(section.kind === "inProgress" && openTurns && openTurns.length > 0) ? (
                  <div style={{ padding: "4px 10px", fontSize: 11, color: "var(--muted)", fontStyle: "italic" }}>
                    {section.kind === "inProgress"
                      ? (isActive !== false && agentStatus === "working"
                          ? <span><BrailleSpinner /> Thinking...</span>
                          : "Waiting")
                      : "(nothing here)"}
                  </div>
                ) : null}
              </>
            ) : null}
          </div>
          </Fragment>
        );
      })}
    </div>
  );
}

/** Tiny inline braille spinner used in placeholder/empty states. The full
 *  10-frame braille cycle ticked at ~80ms/frame is the same animation
 *  Claude Code's TTY shows during a turn, so the visual reads the same
 *  here as in the agent terminal. Self-contained so the In Progress
 *  empty-state can render it without pulling a spinner library. */
const BRAILLE_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
function BrailleSpinner() {
  const [frame, setFrame] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setFrame((f) => (f + 1) % BRAILLE_FRAMES.length), 80);
    return () => clearInterval(id);
  }, []);
  return (
    <span
      aria-hidden="true"
      style={{ display: "inline-block", fontFamily: "var(--font-mono)", width: "1ch", color: "var(--accent)" }}
    >
      {BRAILLE_FRAMES[frame]}
    </span>
  );
}

/** Live agent-turn row at the top of the In Progress section: spinner
 *  + the turn's prompt (single line, ellipsized). Observational — the
 *  row is not selectable, draggable, or menu-bearing; it exists so an
 *  active turn is visible in the Work panel even before (or without)
 *  the agent filing a task. Disappears when the Stop hook closes the
 *  turn (PlanPane refetches on `ModelsChanged{v_agent_turn}`). */
function LiveTurnRow({ turn }: { turn: OpenAgentTurn }) {
  const prompt = turn.prompt.trim() || "(agent turn in progress)";
  return (
    <div
      data-testid={`plan-live-turn-${turn.id}`}
      title={prompt}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 6,
        padding: "4px 10px",
        fontSize: 12,
        color: "var(--muted)",
        whiteSpace: "nowrap",
        overflow: "hidden",
      }}
    >
      <BrailleSpinner />
      <span style={{ overflow: "hidden", textOverflow: "ellipsis", fontStyle: "italic" }}>{prompt}</span>
    </div>
  );
}

/** Transient follow-up line inside the Ready section. Italic muted prefix
 *  ↳ follow-up: + the note, plus a single ✕ dismiss button. No status
 *  icon, no drag, no context menu — these aren't durable rows.
 *  See followup-store.ts for the lifecycle. */
function FollowupRow({
  followup,
  onDismiss,
}: {
  followup: ThreadFollowup;
  onDismiss?: (id: string) => void;
}) {
  return (
    <div
      data-testid={`followup-row-${followup.id}`}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 6,
        padding: "3px 10px 3px 22px",
        fontSize: 11,
        fontStyle: "italic",
        color: "var(--muted)",
        userSelect: "none",
      }}
      title={`Follow-up reminder (transient): ${followup.body}`}
    >
      <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        ↳ follow-up: {followup.body}
      </span>
      {onDismiss ? (
        <button
          type="button"
          aria-label="Dismiss follow-up"
          title="Dismiss follow-up"
          onClick={(event) => {
            event.stopPropagation();
            onDismiss(followup.id);
          }}
          data-testid={`followup-dismiss-${followup.id}`}
          style={{
            background: "transparent",
            border: "none",
            color: "var(--muted)",
            cursor: "pointer",
            fontSize: "var(--text-xs)",
            lineHeight: 1,
            padding: "0 4px",
          }}
        >
          ×
        </button>
      ) : null}
    </div>
  );
}

const firstSectionLabelStyle: CSSProperties = {
  ...sectionHeaderStyle,
  borderTop: "none",
};

/**
 * Section header menu button. A single "⋯" icon that opens a popup menu
 * with the section's available commands. Lets headers stay narrow and
 * absorb new commands (future per-section actions) without crowding the
 * row. Reuses the existing ContextMenu component so styling / outside-
 * click / Escape handling all match the right-click menu.
 */
export function SectionHeaderMenu({ items, testId }: { items: MenuItem[]; testId?: string }) {
  const [menuPos, setMenuPos] = useState<{ x: number; y: number } | null>(null);
  // Render the ⋯ button whenever there are any items at all, including
  // disabled ones — ContextMenu greys disabled items so users can see
  // the command exists and why it isn't currently applicable.
  if (items.length === 0) return null;
  return (
    <>
      <button
        type="button"
        data-testid={testId}
        aria-label="Section actions"
        title="Section actions"
        onClick={(event) => {
          event.stopPropagation();
          const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
          setMenuPos({ x: rect.right, y: rect.bottom + 2 });
        }}
        style={sectionActionButtonStyle}
      >
        ⋯
      </button>
      {menuPos ? (
        <ContextMenu
          items={items}
          position={menuPos}
          onClose={() => setMenuPos(null)}
          minWidth={180}
        />
      ) : null}
    </>
  );
}


function EpicInlineRow({
  rowKey, item, isExpanded, onToggleExpand,
  isSelected, isMarked, isOver, isDragging,
  scopeThreadId, lockInProgress, onSelect, onRequestEdit, onUpdateTask, onOpenMenu, fields,
  onDragStart, onDragEnd, onDragOver, onDragLeave, onDrop,
}: {
  rowKey: string;
  item: WorkItem;
  isExpanded: boolean;
  onToggleExpand(): void;
  isSelected: boolean;
  isMarked: boolean;
  isOver: boolean;
  isDragging: boolean;
  scopeThreadId: string | null;
  lockInProgress?: boolean;
  onSelect?(id: string, modifiers?: { toggle?: boolean; range?: boolean }): void;
  onRequestEdit?(item: WorkItem): void;
  onUpdateTask: (ref: string, changes: TaskDetailChanges) => Promise<void>;
  onOpenMenu(rect: DOMRect, item: WorkItem): void;
  fields: FieldDecl[];
  onDragStart(event: React.DragEvent): void;
  onDragEnd(event: React.DragEvent): void;
  onDragOver(event: React.DragEvent): void;
  onDragLeave(event: React.DragEvent): void;
  onDrop(event: React.DragEvent): void;
}) {
  const dimmed = item.state === "done" || item.state === "canceled";
  const locked = item.state === "in_progress" && (lockInProgress !== false);
  void scopeThreadId;
  return (
    <div
      draggable={!locked}
      onDragStart={locked ? undefined : onDragStart}
      onDragEnd={onDragEnd}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      onClick={(event) => {
        const toggle = event.metaKey || event.ctrlKey;
        const range = event.shiftKey && !toggle;
        if (toggle || range) { onSelect?.(item.ref, { toggle, range }); return; }
        onSelect?.(item.ref);
        onRequestEdit?.(item);
      }}
      onContextMenu={(event) => {
        event.preventDefault();
        onOpenMenu(new DOMRect(event.clientX, event.clientY, 0, 0), item);
      }}
      style={{
        display: "flex", alignItems: "center", gap: 8, padding: "8px 12px",
        cursor: isDragging ? "grabbing" : "pointer",
        borderTop: isOver ? "1px solid var(--accent)" : "1px solid transparent",
        borderLeft: isMarked ? "3px solid var(--status-waiting)" : isSelected ? "3px solid var(--accent)" : "3px solid transparent",
        background: isMarked ? "rgba(217,119,6,0.10)" : isSelected ? "var(--accent-soft-bg)" : isDragging ? "var(--surface-tab-inactive)" : "transparent",
        fontSize: "var(--text-sm)", userSelect: "none", opacity: dimmed ? 0.6 : 1,
      }}
      title={locked ? `${item.title} (in progress — pinned in place)` : item.title}
      data-key={rowKey}
      data-testid={`tasks-row-${item.ref}`}
      data-ref-kind="work_item"
      data-ref-id={item.ref.replace(/^work_item:/, "")}
    >
      <InlineStatusPicker status={item.state} onChange={(state) => { void onUpdateTask(item.ref, { state }); }} locked={locked} />
      <span
        onClick={(event) => { event.stopPropagation(); onToggleExpand(); }}
        style={{ flexShrink: 0, width: 12, textAlign: "center", color: "var(--muted)", fontSize: 10, cursor: "pointer" }}
        title={isExpanded ? "Collapse epic children" : "Expand epic children"}
      >
        {isExpanded ? "\u25BC" : "\u25B6"}
      </span>
      {/* Title is user-selectable so a comment can anchor to it (the
          row is otherwise userSelect:none for clean drag). */}
      <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", fontWeight: "var(--weight-medium)", userSelect: "text" }}>
        {item.title}
      </span>
      <InlineFieldPickers item={item} fields={fields} onUpdateTask={onUpdateTask} />
    </div>
  );
}

function EpicChildrenPane({
  epicRef, children, onReorderTasks, onReparentTask,
  onUpdateTask, onOpenMenu, scopeThreadId, onRequestEdit,
  selectedId, markedIds, onSelect, onAddChildTask, fields,
}: {
  epicRef: string;
  children: WorkItem[];
  onReorderTasks?(refs: string[]): Promise<void>;
  onReparentTask?(ref: string, newParentRef: string | null): Promise<void>;
  onUpdateTask(ref: string, changes: TaskDetailChanges): Promise<void>;
  onOpenMenu(rect: DOMRect, item: WorkItem): void;
  scopeThreadId: string | null;
  onRequestEdit?(item: WorkItem): void;
  selectedId?: string | null;
  markedIds?: ReadonlySet<string>;
  onSelect?(id: string, modifiers?: { toggle?: boolean; range?: boolean }): void;
  onAddChildTask?: (epicRef: string) => void;
  fields: FieldDecl[];
}) {
  const [draggingKey, setDraggingKey] = useState<string | null>(null);
  const [overKey, setOverKey] = useState<string | null>(null);
  const [dropTargetOver, setDropTargetOver] = useState(false);
  const resetDrag = () => { setDraggingKey(null); setOverKey(null); };

  const handleDropOnChild = (targetId: string) => {
    if (draggingKey === null || draggingKey === targetId) { resetDrag(); return; }
    const from = children.findIndex((c) => c.ref === draggingKey);
    const to = children.findIndex((c) => c.ref === targetId);
    if (from < 0 || to < 0) { resetDrag(); return; }
    const next = children.slice();
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved!);
    resetDrag();
    void onReorderTasks?.(next.map((c) => c.ref));
  };

  const handleExternalDrop = (event: React.DragEvent) => {
    event.preventDefault();
    setDropTargetOver(false);
    const drag = decodeWorkItemDrag(event.dataTransfer.getData(WORK_ITEM_DRAG_MIME));
    if (!drag || !onReparentTask) return;
    for (const ref of drag.refs) {
      if (!children.some((c) => c.ref === ref)) void onReparentTask(ref, epicRef);
    }
  };

  return (
    <div style={{ marginLeft: 20, borderLeft: "2px solid var(--border)", paddingLeft: 4 }}>
      {children.map((child) => {
        const key = child.ref;
        const isOver = overKey === key && draggingKey !== key;
        const isDragging = draggingKey === key;
        const isMarked = markedIds?.has(child.ref) ?? false;
        return (
          <InlineItemRow
            key={key}
            rowKey={key}
            item={child}
            isSelected={selectedId === child.ref}
            isMarked={isMarked}
            isOver={isOver}
            isDragging={isDragging}
            scopeThreadId={scopeThreadId}
            onSelect={onSelect}
            onRequestEdit={onRequestEdit}
            onUpdateTask={onUpdateTask}
            onOpenMenu={onOpenMenu}
            fields={fields}
            onDragStart={(event) => {
              // Same drag-cancel workaround as the parent pane's rows —
              // populate dataTransfer first, defer state mutation that
              // would re-render the dragged row in-tick.
              const refs = isMarked && markedIds && markedIds.size > 1 ? [...markedIds] : [child.ref];
              // Only resolve the items we can see locally — this pane
              // only carries the epic's children. Unresolved refs drop
              // through.
              const items = refs
                .map((ref) => children.find((c) => c.ref === ref) ?? null)
                .filter((item): item is WorkItem => item !== null)
                .map((item) => ({ ref: item.ref, title: item.title, state: item.state }));
              setWorkItemDrag(event, { refs, items, fromThreadId: scopeThreadId, parentEpicRef: epicRef });
              queueMicrotask(() => setDraggingKey(key));
            }}
            onDragEnd={resetDrag}
            onDragOver={(event) => {
              if (!draggingKey || draggingKey === key) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
              if (overKey !== key) setOverKey(key);
            }}
            onDragLeave={() => { if (overKey === key) setOverKey(null); }}
            onDrop={(event) => { event.preventDefault(); handleDropOnChild(key); }}
          />
        );
      })}
      <div
        onDragOver={(event) => {
          if (!dragHasWorkItems(event)) return;
          event.preventDefault();
          event.dataTransfer.dropEffect = "move";
          setDropTargetOver(true);
        }}
        onDragLeave={() => setDropTargetOver(false)}
        onDrop={handleExternalDrop}
        style={{
          height: 24, display: "flex", alignItems: "center", paddingLeft: 8,
          fontSize: 10, color: dropTargetOver ? "var(--accent)" : "var(--muted)",
          borderTop: dropTargetOver ? "1px solid var(--accent)" : "1px solid transparent",
          opacity: dropTargetOver ? 1 : 0.5,
        }}
      >
        {children.length === 0 ? "Drop items here to add to epic" : ""}
      </div>
      {onAddChildTask ? (
        <div style={{ padding: "4px 8px 6px" }}>
          <button
            type="button"
            data-testid={`plan-add-child-task-${epicRef}`}
            onClick={() => onAddChildTask(epicRef)}
            style={{ ...miniButtonStyle, fontSize: 11, padding: "2px 8px", color: "var(--muted)" }}
            title="Add a new item inside this epic"
          >
            + Item
          </button>
        </div>
      ) : null}
    </div>
  );
}

/**
 * One collapsed tasks row with inline editing. Title click swaps to an
 * input; status icon and priority marker each open a transparent <select>
 * overlay that commits on change. Clicking anywhere else on the row toggles
 * the expanded TaskDetail (for description + acceptance + delete).
 *
 * Drag-reorder and right-click-to-delete still hang off the outer row div;
 * the inline controls stopPropagation so they don't bubble to the row's
 * expand click.
 */
function InlineItemRow({
  rowKey,
  item,
  isSelected,
  isMarked,
  isOver,
  isDragging,
  scopeThreadId,
  lockInProgress,
  onSelect,
  onRequestEdit,
  onUpdateTask,
  onOpenMenu,
  fields,
  onDragStart,
  onDragEnd,
  onDragOver,
  onDragLeave,
  onDrop,
}: {
  rowKey: string;
  item: WorkItem;
  isSelected: boolean;
  isMarked: boolean;
  isOver: boolean;
  isDragging: boolean;
  scopeThreadId: string | null;
  lockInProgress?: boolean;
  onSelect?(id: string, modifiers?: { toggle?: boolean; range?: boolean }): void;
  onRequestEdit?(item: WorkItem): void;
  onUpdateTask: (ref: string, changes: TaskDetailChanges) => Promise<void>;
  onOpenMenu(rect: DOMRect, item: WorkItem): void;
  fields: FieldDecl[];
  onDragStart(event: React.DragEvent): void;
  onDragEnd(event: React.DragEvent): void;
  onDragOver(event: React.DragEvent): void;
  onDragLeave(event: React.DragEvent): void;
  onDrop(event: React.DragEvent): void;
}) {
  const dimmed = item.state === "done" || item.state === "canceled";
  const locked = item.state === "in_progress" && (lockInProgress !== false);

  // scopeThreadId isn't used directly here, but the outer drag handler that
  // encoded it into dataTransfer was captured at onDragStart creation time —
  // suppress the unused-parameter lint without plumbing it away.
  void scopeThreadId;

  return (
    <div
      draggable={!locked}
      onDragStart={locked ? undefined : onDragStart}
      onDragEnd={onDragEnd}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      onClick={(event) => {
        const toggle = event.metaKey || event.ctrlKey;
        const range = event.shiftKey && !toggle;
        if (toggle || range) {
          onSelect?.(item.ref, { toggle, range });
          return;
        }
        onSelect?.(item.ref);
        onRequestEdit?.(item);
      }}
      onContextMenu={(event) => {
        event.preventDefault();
        onOpenMenu(new DOMRect(event.clientX, event.clientY, 0, 0), item);
      }}
      style={{
        display: "flex",
        alignItems: "center",
        // Let the row shrink below its content width so the flex title
        // span's ellipsis engages and the row never forces its column
        // wider (which used to push/overlap the right Summary panel — tsk144).
        minWidth: 0,
        gap: 8,
        padding: "8px 12px",
        cursor: isDragging ? "grabbing" : "pointer",
        borderTop: isOver ? "1px solid var(--accent)" : "1px solid transparent",
        borderLeft: isMarked
          ? "3px solid var(--status-waiting)"
          : isSelected ? "3px solid var(--accent)" : "3px solid transparent",
        background: isMarked
          ? "rgba(217,119,6,0.10)"
          : isSelected
            ? "var(--accent-soft-bg)"
            : isDragging
              ? "var(--surface-tab-inactive)"
              : "transparent",
        fontSize: "var(--text-sm)",
        userSelect: "none",
        opacity: dimmed ? 0.6 : 1,
      }}
      title={locked ? `${item.title} (in progress — pinned in place)` : item.title}
      data-key={rowKey}
      data-testid={`tasks-row-${item.ref}`}
      data-ref-kind="work_item"
      data-ref-id={item.ref.replace(/^work_item:/, "")}
    >
      <InlineStatusPicker
        status={item.state}
        onChange={(state) => { void onUpdateTask(item.ref, { state }); }}
        locked={locked}
      />
      {/* Title is user-selectable so a comment can anchor to it (the
          row is otherwise userSelect:none for clean drag). */}
      <span
        style={{
          flex: 1,
          minWidth: 0,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
          userSelect: "text",
        }}
      >
        {item.title}
      </span>
      <InlineFieldPickers item={item} fields={fields} onUpdateTask={onUpdateTask} />
    </div>
  );
}

export function InlineStatusPicker({
  status,
  onChange,
  locked,
}: {
  status: CanonicalState;
  onChange(next: CanonicalState): void;
  locked?: boolean;
}) {
  return (
    <span
      onClick={(event) => event.stopPropagation()}
      style={{ position: "relative", display: "inline-block", flexShrink: 0, width: 14, textAlign: "center" }}
      title={locked ? `Status: ${statusLabel(status)} — locked while in progress` : `Status: ${statusLabel(status)} — click to change`}
    >
      <span>{statusIcon(status)}</span>
      {!locked ? (
        <select
          value={status}
          onChange={(event) => onChange(event.target.value as CanonicalState)}
          onClick={(event) => event.stopPropagation()}
          style={{
            position: "absolute",
            inset: 0,
            opacity: 0,
            cursor: "pointer",
            width: "100%",
            height: "100%",
            font: "inherit",
          }}
        >
          {CANONICAL_STATES.map((option) => (
            <option key={option} value={option}>{statusLabel(option)}</option>
          ))}
        </select>
      ) : null}
    </span>
  );
}

/** A row's editable enum fields, each its badge over a transparent
 *  picker (the list's own: priority, …). */
function InlineFieldPickers({
  item,
  fields,
  onUpdateTask,
}: {
  item: WorkItem;
  fields: FieldDecl[];
  onUpdateTask: (ref: string, changes: TaskDetailChanges) => Promise<void>;
}) {
  const shown = fields.filter((f) => f.kind === "enum" && !f.read_only);
  if (shown.length === 0) return null;
  return (
    <>
      {shown.map((field) => {
        const value = fieldText(item.native[field.name]);
        return (
          <span
            key={field.name}
            onClick={(event) => event.stopPropagation()}
            style={{ position: "relative", display: "inline-flex", alignItems: "center", flexShrink: 0, minWidth: 10, minHeight: 10 }}
            title={`${field.title}: ${value || "—"} — click to change`}
          >
            <FieldBadge field={field} value={item.native[field.name]} />
            <select
              aria-label={field.title}
              value={value}
              onChange={(event) => void onUpdateTask(item.ref, { native: { [field.name]: event.target.value } })}
              onClick={(event) => event.stopPropagation()}
              style={{ position: "absolute", inset: 0, opacity: 0, cursor: "pointer", width: "100%", height: "100%", font: "inherit" }}
            >
              {value ? null : <option value="">—</option>}
              {field.values.map((option) => (
                <option key={option} value={option}>{option.replace(/_/g, " ")}</option>
              ))}
            </select>
          </span>
        );
      })}
    </>
  );
}

function StaleEpicChildrenBanner({
  epic,
  staleChildren,
  onCascade,
}: {
  epic: WorkItem;
  staleChildren: WorkItem[];
  onCascade: (targetStatus: CanonicalState) => void;
}) {
  // The classifyEpic rollup will pull this epic back into Ready because
  // its children are still ready/in_progress, so the rail counts will
  // misrepresent the closed state. Surface a one-click cascade fix that
  // mirrors the server-side cascade guard the MCP tools enforce.
  const targetStatus: CanonicalState = epic.state === "blocked" ? "blocked" : "done";
  const n = staleChildren.length;
  const noun = n === 1 ? "child" : "children";
  const label = `Close ${n} ${noun} as ${statusLabel(targetStatus)}`;
  return (
    <div
      data-testid={`stale-epic-children-banner-${epic.ref}`}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "6px 12px",
        margin: "0 0 4px 24px",
        background: "var(--surface-warning, rgba(255,200,0,0.08))",
        border: "1px solid var(--border-warning, rgba(255,200,0,0.4))",
        borderRadius: 4,
        fontSize: "var(--text-xs)",
      }}
    >
      <span style={{ flex: 1, color: "var(--text-warning, var(--fg))" }}>
        Epic closed but {n} {noun} still {n === 1 ? "is" : "are"} pending — rollup will pull it back into Ready.
      </span>
      <button
        type="button"
        onClick={() => onCascade(targetStatus)}
        data-testid={`stale-epic-children-cascade-${epic.ref}`}
        style={{
          padding: "2px 8px",
          fontSize: 11,
          background: "var(--surface-action, #2563eb)",
          color: "var(--text-inverse, white)",
          border: "none",
          borderRadius: 4,
          cursor: "pointer",
        }}
      >
        {label}
      </button>
    </div>
  );
}
