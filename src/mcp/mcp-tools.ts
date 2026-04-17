import type { ToolDef } from "./mcp-server.js";
import type { BatchStore, Batch } from "../persistence/batch-store.js";
import type { Stream } from "../persistence/stream-store.js";
import type { TurnStore } from "../persistence/turn-store.js";
import type { FileChangeStore } from "../persistence/file-change-store.js";
import type {
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
  WorkItemStore,
} from "../persistence/work-item-store.js";

export interface McpToolDeps {
  resolveStream(streamId: string | undefined): Stream;
  resolveBatch(streamId: string, batchId: string): Batch;
  batchStore: BatchStore;
  workItemStore: WorkItemStore;
  turnStore: TurnStore;
  fileChangeStore: FileChangeStore;
}

export function buildWorkItemMcpTools(deps: McpToolDeps): ToolDef[] {
  const { resolveStream, resolveBatch, batchStore, workItemStore, turnStore, fileChangeStore } = deps;

  return [
    {
      name: "newde__get_batch_context",
      description: "Return stream and batch context. Use this to confirm the active batch id before calling work-item tools.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
          batchId: { type: "string", description: "Optional batch id to resolve within the stream." },
        },
      },
      handler: (args: { streamId?: string; batchId?: string }) => {
        const stream = resolveStream(args.streamId);
        const batchState = batchStore.list(stream.id);
        const batch = args.batchId
          ? resolveBatch(stream.id, args.batchId)
          : batchState.batches.find((candidate) => candidate.id === batchState.selectedBatchId) ?? batchState.batches[0] ?? null;
        return {
          streamId: stream.id,
          streamTitle: stream.title,
          batchId: batch?.id ?? null,
          batchTitle: batch?.title ?? null,
          activeBatchId: batchState.activeBatchId,
          selectedBatchId: batchState.selectedBatchId,
          summary: batch?.summary ?? "",
          summaryUpdatedAt: batch?.summary_updated_at ?? null,
        };
      },
    },
    {
      name: "newde__record_batch_summary",
      description:
        "Record a 2-3 sentence rolling summary of what has been happening in this batch. Call this near the end of each turn. Replace the prior summary with one that reflects the overall state plus the latest round's activity.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
          batchId: { type: "string", description: "Required batch id for the batch you are summarising." },
          summary: { type: "string", description: "Required 2-3 sentence rolling summary." },
        },
        required: ["batchId", "summary"],
      },
      handler: (args: { streamId?: string; batchId: string; summary: string }) => {
        const stream = resolveStream(args.streamId);
        resolveBatch(stream.id, args.batchId);
        const updated = batchStore.recordSummary(stream.id, args.batchId, args.summary);
        return { ok: true, summary: updated.summary, summaryUpdatedAt: updated.summary_updated_at };
      },
    },
    {
      name: "newde__list_batch_work",
      description: "List all tracked work items for one batch, grouped by waiting/in progress/done. Always pass the batchId from your session context.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
        },
        required: ["batchId"],
      },
      handler: (args: { streamId?: string; batchId: string }) => {
        const stream = resolveStream(args.streamId);
        resolveBatch(stream.id, args.batchId);
        return workItemStore.getState(args.batchId);
      },
    },
    {
      name: "newde__list_ready_work",
      description: "List actionable work items in one batch that are not blocked by unfinished dependencies. Always pass the batchId from your session context.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
        },
        required: ["batchId"],
      },
      handler: (args: { streamId?: string; batchId: string }) => {
        const stream = resolveStream(args.streamId);
        resolveBatch(stream.id, args.batchId);
        return workItemStore.listReady(args.batchId);
      },
    },
    {
      name: "newde__create_work_item",
      description: "Create a new epic/task/subtask/bug/note within one batch. Always pass the batchId from your session context.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
          parentId: { type: "string", description: "Optional parent epic/task id in the same batch." },
          kind: { type: "string", description: "One of epic, task, subtask, bug, or note." },
          title: { type: "string", description: "Short title for the work item." },
          description: { type: "string", description: "Optional longer description of the approach." },
          acceptanceCriteria: { type: "string", description: "Optional plain-text checklist (one criterion per line) defining observable conditions for 'done'." },
          status: { type: "string", description: "Optional initial status." },
          priority: { type: "string", description: "Optional priority: low, medium, high, or urgent." },
        },
        required: ["batchId", "kind", "title"],
      },
      handler: (args: {
        streamId?: string;
        batchId: string;
        parentId?: string;
        kind: WorkItemKind;
        title: string;
        description?: string;
        acceptanceCriteria?: string | null;
        status?: WorkItemStatus;
        priority?: WorkItemPriority;
      }) => {
        const stream = resolveStream(args.streamId);
        resolveBatch(stream.id, args.batchId);
        const item = workItemStore.createItem({
          batchId: args.batchId,
          parentId: args.parentId,
          kind: args.kind,
          title: args.title,
          description: args.description,
          acceptanceCriteria: args.acceptanceCriteria,
          status: args.status,
          priority: args.priority,
          createdBy: "agent",
          actorId: "mcp",
        });
        return { item, state: workItemStore.getState(args.batchId) };
      },
    },
    {
      name: "newde__update_work_item",
      description: "Update title, description, acceptance criteria, status, priority, or parent of an existing work item in one batch. Always pass the batchId from your session context.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
          itemId: { type: "string", description: "Required id of the work item to update." },
          parentId: { type: "string", description: "Optional new parent epic/task id." },
          title: { type: "string", description: "Optional replacement title." },
          description: { type: "string", description: "Optional replacement description." },
          acceptanceCriteria: { type: "string", description: "Optional replacement acceptance criteria (checklist). Pass empty string to clear." },
          status: { type: "string", description: "Optional replacement status." },
          priority: { type: "string", description: "Optional replacement priority." },
        },
        required: ["batchId", "itemId"],
      },
      handler: (args: {
        streamId?: string;
        batchId: string;
        itemId: string;
        parentId?: string | null;
        title?: string;
        description?: string;
        acceptanceCriteria?: string | null;
        status?: WorkItemStatus;
        priority?: WorkItemPriority;
      }) => {
        const stream = resolveStream(args.streamId);
        resolveBatch(stream.id, args.batchId);
        const item = workItemStore.updateItem({
          batchId: args.batchId,
          itemId: args.itemId,
          parentId: args.parentId,
          title: args.title,
          description: args.description,
          acceptanceCriteria: args.acceptanceCriteria,
          status: args.status,
          priority: args.priority,
          actorKind: "agent",
          actorId: "mcp",
        });
        return { item, state: workItemStore.getState(args.batchId) };
      },
    },
    {
      name: "newde__get_work_item",
      description: "Fetch one work item plus its links (incoming + outgoing) and recent audit events. Use when resuming work on an item and you need the full context without pulling the whole batch.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
          batchId: { type: "string", description: "Required batch id." },
          itemId: { type: "string", description: "Required id of the work item to fetch." },
        },
        required: ["batchId", "itemId"],
      },
      handler: (args: { streamId?: string; batchId: string; itemId: string }) => {
        const stream = resolveStream(args.streamId);
        resolveBatch(stream.id, args.batchId);
        const detail = workItemStore.getItemDetail(args.batchId, args.itemId);
        if (!detail) throw new Error(`unknown work item: ${args.itemId}`);
        return detail;
      },
    },
    {
      name: "newde__delete_work_item",
      description: "Soft-delete a work item. Hidden from lists but preserved in the audit log.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
          itemId: { type: "string", description: "Required id of the work item to delete." },
        },
        required: ["batchId", "itemId"],
      },
      handler: (args: { streamId?: string; batchId: string; itemId: string }) => {
        const stream = resolveStream(args.streamId);
        resolveBatch(stream.id, args.batchId);
        workItemStore.deleteItem(args.batchId, args.itemId, "agent", "mcp");
        return { ok: true, state: workItemStore.getState(args.batchId) };
      },
    },
    {
      name: "newde__reorder_work_items",
      description: "Reorder sibling work items within a batch. All ids must share the same parent.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
          orderedItemIds: {
            type: "array",
            description: "Full list of sibling work item ids in the desired new order.",
            items: { type: "string" },
          },
        },
        required: ["batchId", "orderedItemIds"],
      },
      handler: (args: { streamId?: string; batchId: string; orderedItemIds: string[] }) => {
        const stream = resolveStream(args.streamId);
        resolveBatch(stream.id, args.batchId);
        workItemStore.reorderItems(args.batchId, args.orderedItemIds, "agent", "mcp");
        return { ok: true, state: workItemStore.getState(args.batchId) };
      },
    },
    {
      name: "newde__link_work_items",
      description:
        "Create a relationship between two work items in one batch. linkType is one of: " +
        "`blocks` (from-item must finish before to-item starts), " +
        "`relates_to` (general association), " +
        "`discovered_from` (from-item was uncovered while working on to-item; preferred for scope-creep escape), " +
        "`duplicates` (from-item is the same work as to-item — close the dupe), " +
        "`supersedes` (from-item replaces to-item — the older one is stale), " +
        "`replies_to` (from-item is a threaded note/response to to-item).",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
          fromItemId: { type: "string", description: "Source work item id." },
          toItemId: { type: "string", description: "Target work item id." },
          linkType: { type: "string", description: "One of blocks, relates_to, discovered_from, duplicates, supersedes, replies_to." },
        },
        required: ["batchId", "fromItemId", "toItemId", "linkType"],
      },
      handler: (args: {
        streamId?: string;
        batchId: string;
        fromItemId: string;
        toItemId: string;
        linkType: "blocks" | "relates_to" | "discovered_from" | "duplicates" | "supersedes" | "replies_to";
      }) => {
        const stream = resolveStream(args.streamId);
        resolveBatch(stream.id, args.batchId);
        workItemStore.linkItems(args.batchId, args.fromItemId, args.toItemId, args.linkType);
        return { ok: true };
      },
    },
    {
      name: "newde__list_agent_turn",
      description:
        "List recent agent turns for a batch (newest first). Each turn represents one user prompt and the agent's Stop-terminated response, with the snapshot summary and optionally a single in-progress work item it was attributed to.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
          batchId: { type: "string", description: "Required batch id." },
          limit: { type: "number", description: "Optional cap on the number of turns returned (default 50)." },
        },
        required: ["batchId"],
      },
      handler: (args: { streamId?: string; batchId: string; limit?: number }) => {
        const stream = resolveStream(args.streamId);
        resolveBatch(stream.id, args.batchId);
        return turnStore.listForBatch(args.batchId, args.limit);
      },
    },
    {
      name: "newde__list_batch_file_change",
      description:
        "List recent file changes recorded for a batch (newest first). Each row shows path, change_kind (created/updated/deleted), source (hook or fs-watch), and optional turn_id / work_item_id attribution.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
          batchId: { type: "string", description: "Required batch id." },
          limit: { type: "number", description: "Optional cap (default 200)." },
        },
        required: ["batchId"],
      },
      handler: (args: { streamId?: string; batchId: string; limit?: number }) => {
        const stream = resolveStream(args.streamId);
        resolveBatch(stream.id, args.batchId);
        return fileChangeStore.listForBatch(args.batchId, args.limit);
      },
    },
    {
      name: "newde__add_work_note",
      description: "Append a note/history entry to a work item in one batch. Always pass the batchId from your session context.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
          itemId: { type: "string", description: "Optional work item id if the note belongs to one specific item." },
          note: { type: "string", description: "Required note content." },
        },
        required: ["batchId", "note"],
      },
      handler: (args: { streamId?: string; batchId: string; itemId?: string; note: string }) => {
        const stream = resolveStream(args.streamId);
        resolveBatch(stream.id, args.batchId);
        workItemStore.addNote(args.batchId, args.itemId ?? null, args.note, "agent", "mcp");
        return {
          ok: true,
          events: workItemStore.listEvents(args.batchId, args.itemId),
        };
      },
    },
  ];
}
