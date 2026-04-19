import type { Batch } from "../persistence/batch-store.js";
import type { AgentTurn, TurnStore } from "../persistence/turn-store.js";
import type { BatchFileChange, FileChangeStore } from "../persistence/file-change-store.js";
import type {
  BatchWorkState,
  WorkItem,
  WorkItemDetail,
  WorkItemEvent,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
  WorkItemStore,
} from "../persistence/work-item-store.js";

export interface BacklogState {
  items: WorkItem[];
  waiting: WorkItem[];
  inProgress: WorkItem[];
  done: WorkItem[];
}

export interface WorkItemApiDeps {
  resolveBatch(streamId: string, batchId: string): Batch;
  workItemStore: WorkItemStore;
  turnStore: TurnStore;
  fileChangeStore: FileChangeStore;
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

export interface WorkItemApi {
  getBatchWorkState(streamId: string, batchId: string): BatchWorkState;
  getWorkItem(streamId: string, batchId: string, itemId: string): WorkItemDetail | null;
  createWorkItem(streamId: string, batchId: string, input: CreateWorkItemInput): BatchWorkState;
  updateWorkItem(streamId: string, batchId: string, itemId: string, changes: UpdateWorkItemChanges): BatchWorkState;
  deleteWorkItem(streamId: string, batchId: string, itemId: string): BatchWorkState;
  reorderWorkItems(streamId: string, batchId: string, orderedItemIds: string[]): BatchWorkState;
  moveWorkItemToBatch(streamId: string, fromBatchId: string, itemId: string, toBatchId: string): { from: BatchWorkState; to: BatchWorkState };
  getBacklogState(): BacklogState;
  createBacklogItem(input: CreateWorkItemInput): BacklogState;
  updateBacklogItem(itemId: string, changes: UpdateWorkItemChanges): BacklogState;
  deleteBacklogItem(itemId: string): BacklogState;
  reorderBacklog(orderedItemIds: string[]): BacklogState;
  moveWorkItemToBacklog(streamId: string, fromBatchId: string, itemId: string): { from: BatchWorkState; backlog: BacklogState };
  moveBacklogItemToBatch(streamId: string, itemId: string, toBatchId: string): { backlog: BacklogState; to: BatchWorkState };
  addWorkItemNote(streamId: string, batchId: string, itemId: string, note: string): WorkItemEvent[];
  listWorkItemEvents(streamId: string, batchId: string, itemId?: string): WorkItemEvent[];
  listAgentTurns(streamId: string, batchId: string, limit?: number): AgentTurn[];
  listFileChanges(streamId: string, batchId: string, limit?: number): BatchFileChange[];
}

function buildBacklogState(store: WorkItemStore): BacklogState {
  const items = store.listBacklog();
  return {
    items,
    waiting: items.filter((item) => item.status === "waiting" || item.status === "ready" || item.status === "blocked"),
    inProgress: items.filter((item) => item.status === "in_progress" || item.status === "human_check"),
    done: items.filter((item) => item.status === "done" || item.status === "canceled"),
  };
}

export function createWorkItemApi({ resolveBatch, workItemStore, turnStore, fileChangeStore }: WorkItemApiDeps): WorkItemApi {
  return {
    getBatchWorkState(streamId, batchId) {
      resolveBatch(streamId, batchId);
      return workItemStore.getState(batchId);
    },

    getWorkItem(streamId, batchId, itemId) {
      resolveBatch(streamId, batchId);
      return workItemStore.getItemDetail(batchId, itemId);
    },

    createWorkItem(streamId, batchId, input) {
      resolveBatch(streamId, batchId);
      workItemStore.createItem({
        batchId,
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
      return workItemStore.getState(batchId);
    },

    updateWorkItem(streamId, batchId, itemId, changes) {
      resolveBatch(streamId, batchId);
      workItemStore.updateItem({
        batchId,
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
      return workItemStore.getState(batchId);
    },

    deleteWorkItem(streamId, batchId, itemId) {
      resolveBatch(streamId, batchId);
      workItemStore.deleteItem(batchId, itemId, "user", "ui");
      return workItemStore.getState(batchId);
    },

    reorderWorkItems(streamId, batchId, orderedItemIds) {
      resolveBatch(streamId, batchId);
      workItemStore.reorderItems(batchId, orderedItemIds, "user", "ui");
      return workItemStore.getState(batchId);
    },

    moveWorkItemToBatch(streamId, fromBatchId, itemId, toBatchId) {
      resolveBatch(streamId, fromBatchId);
      resolveBatch(streamId, toBatchId);
      workItemStore.moveItemToBatch(fromBatchId, itemId, toBatchId, "user", "ui");
      return {
        from: workItemStore.getState(fromBatchId),
        to: workItemStore.getState(toBatchId),
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

    moveWorkItemToBacklog(streamId, fromBatchId, itemId) {
      resolveBatch(streamId, fromBatchId);
      workItemStore.moveItemToScope(fromBatchId, itemId, null, "user", "ui");
      return {
        from: workItemStore.getState(fromBatchId),
        backlog: buildBacklogState(workItemStore),
      };
    },

    moveBacklogItemToBatch(streamId, itemId, toBatchId) {
      resolveBatch(streamId, toBatchId);
      workItemStore.moveItemToScope(null, itemId, toBatchId, "user", "ui");
      return {
        backlog: buildBacklogState(workItemStore),
        to: workItemStore.getState(toBatchId),
      };
    },

    addWorkItemNote(streamId, batchId, itemId, note) {
      resolveBatch(streamId, batchId);
      workItemStore.addNote(batchId, itemId, note, "user", "ui");
      return workItemStore.listEvents(batchId, itemId);
    },

    listWorkItemEvents(streamId, batchId, itemId) {
      resolveBatch(streamId, batchId);
      return workItemStore.listEvents(batchId, itemId);
    },

    listAgentTurns(streamId, batchId, limit) {
      resolveBatch(streamId, batchId);
      return turnStore.listForBatch(batchId, limit);
    },

    listFileChanges(streamId, batchId, limit) {
      resolveBatch(streamId, batchId);
      return fileChangeStore.listForBatch(batchId, limit);
    },
  };
}
