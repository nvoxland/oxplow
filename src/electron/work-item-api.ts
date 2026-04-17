import type { Batch } from "../persistence/batch-store.js";
import type {
  BatchWorkState,
  WorkItemDetail,
  WorkItemEvent,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
  WorkItemStore,
} from "../persistence/work-item-store.js";

export interface WorkItemApiDeps {
  resolveBatch(streamId: string, batchId: string): Batch;
  workItemStore: WorkItemStore;
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
  addWorkItemNote(streamId: string, batchId: string, itemId: string, note: string): WorkItemEvent[];
  listWorkItemEvents(streamId: string, batchId: string, itemId?: string): WorkItemEvent[];
}

export function createWorkItemApi({ resolveBatch, workItemStore }: WorkItemApiDeps): WorkItemApi {
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

    addWorkItemNote(streamId, batchId, itemId, note) {
      resolveBatch(streamId, batchId);
      workItemStore.addNote(batchId, itemId, note, "user", "ui");
      return workItemStore.listEvents(batchId, itemId);
    },

    listWorkItemEvents(streamId, batchId, itemId) {
      resolveBatch(streamId, batchId);
      return workItemStore.listEvents(batchId, itemId);
    },
  };
}
