import type { Thread } from "../persistence/thread-store.js";
import type { AgentTurn, TurnStore } from "../persistence/turn-store.js";
import type {
  WorkItemEffort,
  WorkItemEffortStore,
} from "../persistence/work-item-effort-store.js";
import type { FileSnapshot, SnapshotStore } from "../persistence/snapshot-store.js";
import type {
  ThreadWorkState,
  WorkItem,
  WorkItemDetail,
  WorkItemEvent,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
  WorkItemStore,
  WorkNote,
} from "../persistence/work-item-store.js";

export interface BacklogState {
  items: WorkItem[];
  waiting: WorkItem[];
  inProgress: WorkItem[];
  done: WorkItem[];
}

export interface WorkItemApiDeps {
  resolveThread(streamId: string, threadId: string): Thread;
  workItemStore: WorkItemStore;
  turnStore: TurnStore;
  effortStore: WorkItemEffortStore;
  snapshotStore: SnapshotStore;
}

export interface CreateWorkItemInput {
  kind: WorkItemKind;
  title: string;
  description?: string;
  acceptanceCriteria?: string | null;
  parentId?: string | null;
  status?: WorkItemStatus;
  priority?: WorkItemPriority;
}

export interface UpdateWorkItemChanges {
  title?: string;
  description?: string;
  acceptanceCriteria?: string | null;
  parentId?: string | null;
  status?: WorkItemStatus;
  priority?: WorkItemPriority;
}

export interface EffortDetail {
  effort: WorkItemEffort;
  start_snapshot: FileSnapshot | null;
  end_snapshot: FileSnapshot | null;
  turn_ids: string[];
  /** Paths that differ between start and end snapshots (empty when either
   *  side is null or hashes are identical). */
  changed_paths: string[];
  /** Per-category counts for the Efforts panel — mirrors the shape used
   *  by SnapshotSummary so the modal can render the same +/~/− row as
   *  Git/Local History. Zeroed when there's no snapshot pair. */
  counts: { created: number; updated: number; deleted: number };
}

export interface WorkItemApi {
  getThreadWorkState(streamId: string, threadId: string): ThreadWorkState;
  getWorkItem(streamId: string, threadId: string, itemId: string): WorkItemDetail | null;
  createWorkItem(streamId: string, threadId: string, input: CreateWorkItemInput): ThreadWorkState;
  updateWorkItem(streamId: string, threadId: string, itemId: string, changes: UpdateWorkItemChanges): ThreadWorkState;
  deleteWorkItem(streamId: string, threadId: string, itemId: string): ThreadWorkState;
  reorderWorkItems(streamId: string, threadId: string, orderedItemIds: string[]): ThreadWorkState;
  moveWorkItemToThread(streamId: string, fromThreadId: string, itemId: string, toThreadId: string, toStreamId?: string): { from: ThreadWorkState; to: ThreadWorkState };
  getBacklogState(): BacklogState;
  createBacklogItem(input: CreateWorkItemInput): BacklogState;
  updateBacklogItem(itemId: string, changes: UpdateWorkItemChanges): BacklogState;
  deleteBacklogItem(itemId: string): BacklogState;
  reorderBacklog(orderedItemIds: string[]): BacklogState;
  moveWorkItemToBacklog(streamId: string, fromThreadId: string, itemId: string): { from: ThreadWorkState; backlog: BacklogState };
  moveBacklogItemToThread(streamId: string, itemId: string, toThreadId: string): { backlog: BacklogState; to: ThreadWorkState };
  addWorkItemNote(streamId: string, threadId: string, itemId: string, note: string): WorkItemEvent[];
  listWorkItemEvents(streamId: string, threadId: string, itemId?: string): WorkItemEvent[];
  getWorkNotes(itemId: string): WorkNote[];
  listAgentTurns(streamId: string, threadId: string, limit?: number): AgentTurn[];
  listWorkItemEfforts(itemId: string): EffortDetail[];
}

function buildBacklogState(store: WorkItemStore): BacklogState {
  const items = store.listBacklog();
  return {
    items,
    waiting: items.filter((item) => item.status === "ready" || item.status === "blocked"),
    inProgress: items.filter((item) => item.status === "in_progress" || item.status === "human_check"),
    done: items.filter((item) => item.status === "done" || item.status === "canceled" || item.status === "archived"),
  };
}

function computeEffortDiff(
  snapshotStore: SnapshotStore,
  startId: string | null,
  endId: string | null,
): { changed_paths: string[]; counts: EffortDetail["counts"] } {
  const empty = { changed_paths: [], counts: { created: 0, updated: 0, deleted: 0 } };
  if (!startId || !endId || startId === endId) return empty;
  const summary = snapshotStore.getSnapshotSummary(endId, startId);
  if (!summary) return empty;
  return {
    changed_paths: Object.keys(summary.files).sort(),
    counts: summary.counts,
  };
}

function buildEffortDetail(
  effort: WorkItemEffort,
  effortStore: WorkItemEffortStore,
  snapshotStore: SnapshotStore,
): EffortDetail {
  const start_snapshot = effort.start_snapshot_id
    ? snapshotStore.getSnapshot(effort.start_snapshot_id)
    : null;
  const end_snapshot = effort.end_snapshot_id
    ? snapshotStore.getSnapshot(effort.end_snapshot_id)
    : null;
  const diff = computeEffortDiff(snapshotStore, effort.start_snapshot_id, effort.end_snapshot_id);
  return {
    effort,
    start_snapshot,
    end_snapshot,
    turn_ids: effortStore.listTurnsForEffort(effort.id),
    changed_paths: diff.changed_paths,
    counts: diff.counts,
  };
}

export function createWorkItemApi({
  resolveThread,
  workItemStore,
  turnStore,
  effortStore,
  snapshotStore,
}: WorkItemApiDeps): WorkItemApi {
  return {
    getThreadWorkState(streamId, threadId) {
      resolveThread(streamId, threadId);
      return workItemStore.getState(threadId);
    },

    getWorkItem(streamId, threadId, itemId) {
      resolveThread(streamId, threadId);
      return workItemStore.getItemDetail(threadId, itemId);
    },

    createWorkItem(streamId, threadId, input) {
      resolveThread(streamId, threadId);
      workItemStore.createItem({
        threadId,
        parentId: input.parentId,
        kind: input.kind,
        title: input.title,
        description: input.description,
        acceptanceCriteria: input.acceptanceCriteria,
        status: input.status,
        priority: input.priority,
        createdBy: "user",
        actorId: "ui",
      });
      return workItemStore.getState(threadId);
    },

    updateWorkItem(streamId, threadId, itemId, changes) {
      resolveThread(streamId, threadId);
      workItemStore.updateItem({
        threadId,
        itemId,
        title: changes.title,
        description: changes.description,
        acceptanceCriteria: changes.acceptanceCriteria,
        parentId: changes.parentId,
        status: changes.status,
        priority: changes.priority,
        actorKind: "user",
        actorId: "ui",
      });
      return workItemStore.getState(threadId);
    },

    deleteWorkItem(streamId, threadId, itemId) {
      resolveThread(streamId, threadId);
      workItemStore.deleteItem(threadId, itemId, "user", "ui");
      return workItemStore.getState(threadId);
    },

    reorderWorkItems(streamId, threadId, orderedItemIds) {
      resolveThread(streamId, threadId);
      workItemStore.reorderItems(threadId, orderedItemIds, "user", "ui");
      return workItemStore.getState(threadId);
    },

    moveWorkItemToThread(streamId, fromThreadId, itemId, toThreadId, toStreamId) {
      resolveThread(streamId, fromThreadId);
      resolveThread(toStreamId ?? streamId, toThreadId);
      workItemStore.moveItemToThread(fromThreadId, itemId, toThreadId, "user", "ui");
      return {
        from: workItemStore.getState(fromThreadId),
        to: workItemStore.getState(toThreadId),
      };
    },

    getBacklogState() {
      return buildBacklogState(workItemStore);
    },

    createBacklogItem(input) {
      workItemStore.createBacklogItem({
        kind: input.kind,
        title: input.title,
        description: input.description,
        acceptanceCriteria: input.acceptanceCriteria,
        status: input.status,
        priority: input.priority,
        createdBy: "user",
        actorId: "ui",
      });
      return buildBacklogState(workItemStore);
    },

    updateBacklogItem(itemId, changes) {
      workItemStore.updateBacklogItem({
        itemId,
        title: changes.title,
        description: changes.description,
        acceptanceCriteria: changes.acceptanceCriteria,
        status: changes.status,
        priority: changes.priority,
        actorKind: "user",
        actorId: "ui",
      });
      return buildBacklogState(workItemStore);
    },

    deleteBacklogItem(itemId) {
      workItemStore.deleteBacklogItem(itemId, "user", "ui");
      return buildBacklogState(workItemStore);
    },

    reorderBacklog(orderedItemIds) {
      workItemStore.reorderBacklog(orderedItemIds, "user", "ui");
      return buildBacklogState(workItemStore);
    },

    moveWorkItemToBacklog(streamId, fromThreadId, itemId) {
      resolveThread(streamId, fromThreadId);
      workItemStore.moveItemToScope(fromThreadId, itemId, null, "user", "ui");
      return {
        from: workItemStore.getState(fromThreadId),
        backlog: buildBacklogState(workItemStore),
      };
    },

    moveBacklogItemToThread(streamId, itemId, toThreadId) {
      resolveThread(streamId, toThreadId);
      workItemStore.moveItemToScope(null, itemId, toThreadId, "user", "ui");
      return {
        backlog: buildBacklogState(workItemStore),
        to: workItemStore.getState(toThreadId),
      };
    },

    addWorkItemNote(streamId, threadId, itemId, note) {
      resolveThread(streamId, threadId);
      workItemStore.addNote(threadId, itemId, note, "user", "ui");
      return workItemStore.listEvents(threadId, itemId);
    },

    listWorkItemEvents(streamId, threadId, itemId) {
      resolveThread(streamId, threadId);
      return workItemStore.listEvents(threadId, itemId);
    },

    getWorkNotes(itemId) {
      return workItemStore.getWorkNotes(itemId);
    },

    listAgentTurns(streamId, threadId, limit) {
      resolveThread(streamId, threadId);
      return turnStore.listForThread(threadId, limit);
    },

    listWorkItemEfforts(itemId) {
      return effortStore
        .listEffortsForWorkItem(itemId)
        .map((effort) => buildEffortDetail(effort, effortStore, snapshotStore));
    },
  };
}
