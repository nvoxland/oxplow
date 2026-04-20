import type { ToolDef } from "./mcp-server.js";
import type { BatchStore, Batch } from "../persistence/batch-store.js";
import type { Stream, StreamStore } from "../persistence/stream-store.js";
import type { TurnStore } from "../persistence/turn-store.js";
import type { FileChangeStore } from "../persistence/file-change-store.js";
import type {
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
  WorkItemStore,
} from "../persistence/work-item-store.js";
import type { CommitPoint, CommitPointStore } from "../persistence/commit-point-store.js";

export interface McpToolDeps {
  resolveStream(streamId: string | undefined): Stream;
  /** Batch-id-only lookup. Tools accept `batchId` alone; streamId is derived
   *  from the batch row. Handles the case where the agent's prompt
   *  streamId drifted out of sync with reality (the old `resolveBatch`
   *  required both args and threw). */
  resolveBatchById(batchId: string): Batch;
  batchStore: BatchStore;
  streamStore: StreamStore;
  workItemStore: WorkItemStore;
  commitPointStore: CommitPointStore;
  /** Synchronously run `git commit` for a commit point (message is the final
   *  text the user approved). Wired by the runtime to the batch queue
   *  orchestrator; thrown errors bubble back to the agent so it can retry. */
  executeCommit(cpId: string, message: string): CommitPoint;
  turnStore: TurnStore;
  fileChangeStore: FileChangeStore;
}

export function hasAcceptanceCriteria(raw: string | null | undefined): boolean {
  return typeof raw === "string" && raw.trim().length > 0;
}

// Returns true when the description reads like it's hiding the acceptance
// checklist — specifically the literal phrase "acceptance criteria" AND at
// least one bullet-looking line. The second gate keeps legitimate
// discussion-style descriptions (e.g. "the existing acceptance criteria
// said …") from tripping the guard.
export function descriptionLooksLikeEmbeddedCriteria(description: string | null | undefined): boolean {
  if (typeof description !== "string" || description.length === 0) return false;
  if (!/acceptance criteria/i.test(description)) return false;
  return /^\s*[-*]\s+\S/m.test(description);
}

export function buildWorkItemMcpTools(deps: McpToolDeps): ToolDef[] {
  const { resolveStream, resolveBatchById, batchStore, streamStore, workItemStore, commitPointStore, executeCommit, turnStore, fileChangeStore } = deps;

  // Prefer the batch row's own stream_id over whatever streamId the caller
  // passed (or didn't). Returns { batch, stream } — both guaranteed to agree
  // on stream_id. Throws "unknown batch: …" if the batchId doesn't exist.
  function resolveBatchAndStream(args: { streamId?: string; batchId: string }): { batch: Batch; stream: Stream } {
    const batch = resolveBatchById(args.batchId);
    const stream = resolveStream(batch.stream_id);
    return { batch, stream };
  }

  // Mutation tools return this instead of the full BatchWorkState to keep
  // agent context lean — a full state dump per call was burning tokens for
  // data the agent already has (it just did the mutation).
  function counts(batchId: string): { waiting: number; inProgress: number; done: number; ready: number } {
    const state = workItemStore.getState(batchId);
    const ready = state.waiting.filter((item) => item.status === "ready").length;
    return {
      waiting: state.waiting.length,
      inProgress: state.inProgress.length,
      done: state.done.length,
      ready,
    };
  }

  return [
    {
      name: "newde__get_batch_context",
      description: "Return stream and batch context. Use this to confirm the active batch id before calling work-item tools.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from batchId when omitted; only passed explicitly for the no-batchId form of get_batch_context." },
          batchId: { type: "string", description: "Optional batch id to resolve within the stream." },
        },
      },
      handler: (args: { streamId?: string; batchId?: string }) => {
        // When the caller names a batchId, derive stream from the batch row
        // itself — the agent's prompt streamId may have drifted. Without
        // batchId, fall back to the current-stream default.
        const { stream, batch: explicitBatch } = args.batchId
          ? (() => {
              const b = resolveBatchById(args.batchId!);
              return { stream: resolveStream(b.stream_id), batch: b };
            })()
          : { stream: resolveStream(args.streamId), batch: null as Batch | null };
        const batchState = batchStore.list(stream.id);
        const batch = explicitBatch
          ?? batchState.batches.find((candidate) => candidate.id === batchState.selectedBatchId)
          ?? batchState.batches[0]
          ?? null;
        // Cross-stream snapshot — lets the agent notice that "current
        // stream" may have drifted from where it actually writes. Each
        // entry is the would-be active batch in a peer stream (falling
        // back to the first batch if nothing's active yet).
        const otherActiveBatches = streamStore.list()
          .filter((s) => s.id !== stream.id)
          .map((peer) => {
            const peerState = batchStore.list(peer.id);
            const peerActive = peerState.batches.find((b) => b.id === peerState.activeBatchId)
              ?? peerState.batches[0]
              ?? null;
            return {
              streamId: peer.id,
              streamTitle: peer.title,
              batchId: peerActive?.id ?? null,
              batchTitle: peerActive?.title ?? null,
              activeBatchId: peerState.activeBatchId,
            };
          });
        return {
          streamId: stream.id,
          streamTitle: stream.title,
          batchId: batch?.id ?? null,
          batchTitle: batch?.title ?? null,
          activeBatchId: batchState.activeBatchId,
          selectedBatchId: batchState.selectedBatchId,
          otherActiveBatches,
        };
      },
    },
    {
      name: "newde__list_batch_work",
      description: "List all tracked work items for one batch, grouped by waiting/in progress/done. Always pass the batchId from your session context.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from batchId when omitted; only passed explicitly for the no-batchId form of get_batch_context." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
        },
        required: ["batchId"],
      },
      handler: (args: { streamId?: string; batchId: string }) => {
        resolveBatchAndStream(args);
        return workItemStore.getState(args.batchId);
      },
    },
    {
      name: "newde__list_ready_work",
      description: "List actionable work items in one batch that are not blocked by unfinished dependencies. Always pass the batchId from your session context.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from batchId when omitted; only passed explicitly for the no-batchId form of get_batch_context." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
        },
        required: ["batchId"],
      },
      handler: (args: { streamId?: string; batchId: string }) => {
        resolveBatchAndStream(args);
        return workItemStore.listReady(args.batchId);
      },
    },
    {
      name: "newde__create_work_item",
      description: "Create a new epic/task/subtask/bug/note within one batch. Always pass the batchId from your session context. acceptanceCriteria, priority, and parentId are top-level JSON fields — do not embed them inside description as XML-style tags.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from batchId when omitted; only passed explicitly for the no-batchId form of get_batch_context." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
          parentId: { type: "string", description: "Optional parent epic/task id in the same batch." },
          kind: { type: "string", description: "One of epic, task, subtask, bug, or note." },
          title: { type: "string", description: "Short title for the work item." },
          description: { type: "string", description: "Optional longer description of the approach." },
          acceptanceCriteria: { type: "string", description: "Optional plain-text checklist (one criterion per line) defining observable conditions for 'done'." },
          status: { type: "string", description: "Optional initial status. One of: ready, in_progress, human_check, blocked, done, canceled, archived." },
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
        resolveBatchAndStream(args);
        // Silent-failure guard: agents sometimes cram the acceptance
        // checklist into `description` instead of the dedicated top-level
        // `acceptanceCriteria` field. The DB accepts it either way, so the
        // mistake shows up only as a UI gap (the Work panel's acceptance
        // column goes empty). Returning a soft error forces a re-call with
        // the criteria promoted to the proper field.
        if (!hasAcceptanceCriteria(args.acceptanceCriteria) && descriptionLooksLikeEmbeddedCriteria(args.description)) {
          return {
            error: "acceptanceCriteria is a top-level JSON field; don't embed it inside description. Re-call newde__create_work_item with the checklist in the acceptanceCriteria field (one criterion per line, plain text).",
          };
        }
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
        return { item, counts: counts(args.batchId) };
      },
    },
    {
      name: "newde__update_work_item",
      description: "Update title, description, acceptance criteria, status, priority, or parent of an existing work item in one batch. Always pass the batchId from your session context.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from batchId when omitted; only passed explicitly for the no-batchId form of get_batch_context." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
          itemId: { type: "string", description: "Required id of the work item to update." },
          parentId: { type: "string", description: "Optional new parent epic/task id." },
          title: { type: "string", description: "Optional replacement title." },
          description: { type: "string", description: "Optional replacement description." },
          acceptanceCriteria: { type: "string", description: "Optional replacement acceptance criteria (checklist). Pass empty string to clear." },
          status: { type: "string", description: "Optional replacement status. One of: ready, in_progress, human_check, blocked, done, canceled, archived. When you believe a task is complete, set status to 'human_check' — never set 'done' yourself; the user marks 'done' after reviewing. 'archived' hides the item from the default Work view." },
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
        resolveBatchAndStream(args);
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
        return { item, counts: counts(args.batchId) };
      },
    },
    {
      name: "newde__get_work_item",
      description: "Fetch one work item plus its links (incoming + outgoing) and recent audit events. Use when resuming work on an item and you need the full context without pulling the whole batch.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from batchId when omitted; only passed explicitly for the no-batchId form of get_batch_context." },
          batchId: { type: "string", description: "Required batch id." },
          itemId: { type: "string", description: "Required id of the work item to fetch." },
        },
        required: ["batchId", "itemId"],
      },
      handler: (args: { streamId?: string; batchId: string; itemId: string }) => {
        resolveBatchAndStream(args);
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
          streamId: { type: "string", description: "Optional. Server infers the owning stream from batchId when omitted; only passed explicitly for the no-batchId form of get_batch_context." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
          itemId: { type: "string", description: "Required id of the work item to delete." },
        },
        required: ["batchId", "itemId"],
      },
      handler: (args: { streamId?: string; batchId: string; itemId: string }) => {
        resolveBatchAndStream(args);
        workItemStore.deleteItem(args.batchId, args.itemId, "agent", "mcp");
        return { ok: true, counts: counts(args.batchId) };
      },
    },
    {
      name: "newde__reorder_work_items",
      description: "Reorder sibling work items within a batch. All ids must share the same parent.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from batchId when omitted; only passed explicitly for the no-batchId form of get_batch_context." },
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
        resolveBatchAndStream(args);
        workItemStore.reorderItems(args.batchId, args.orderedItemIds, "agent", "mcp");
        return { ok: true, counts: counts(args.batchId) };
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
          streamId: { type: "string", description: "Optional. Server infers the owning stream from batchId when omitted; only passed explicitly for the no-batchId form of get_batch_context." },
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
        resolveBatchAndStream(args);
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
          streamId: { type: "string", description: "Optional. Server infers the owning stream from batchId when omitted; only passed explicitly for the no-batchId form of get_batch_context." },
          batchId: { type: "string", description: "Required batch id." },
          limit: { type: "number", description: "Optional cap on the number of turns returned (default 50)." },
        },
        required: ["batchId"],
      },
      handler: (args: { streamId?: string; batchId: string; limit?: number }) => {
        resolveBatchAndStream(args);
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
          streamId: { type: "string", description: "Optional. Server infers the owning stream from batchId when omitted; only passed explicitly for the no-batchId form of get_batch_context." },
          batchId: { type: "string", description: "Required batch id." },
          limit: { type: "number", description: "Optional cap (default 200)." },
        },
        required: ["batchId"],
      },
      handler: (args: { streamId?: string; batchId: string; limit?: number }) => {
        resolveBatchAndStream(args);
        return fileChangeStore.listForBatch(args.batchId, args.limit);
      },
    },
    {
      name: "newde__add_work_note",
      description: "Append a note/history entry to a work item in one batch. Always pass the batchId from your session context.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from batchId when omitted; only passed explicitly for the no-batchId form of get_batch_context." },
          batchId: { type: "string", description: "Required batch id for the work you are managing." },
          itemId: { type: "string", description: "Optional work item id if the note belongs to one specific item." },
          note: { type: "string", description: "Required note content." },
        },
        required: ["batchId", "note"],
      },
      handler: (args: { streamId?: string; batchId: string; itemId?: string; note: string }) => {
        resolveBatchAndStream(args);
        workItemStore.addNote(args.batchId, args.itemId ?? null, args.note, "agent", "mcp");
        return { ok: true };
      },
    },
    {
      name: "newde__propose_commit",
      description:
        "Record (or re-record) a drafted commit message for an active commit point. Call this as soon as you've drafted a message, THEN output the message in your reply and ASK the user to approve. Do NOT run `git add` or `git commit` yourself, and do NOT call `newde__commit` until the user has approved. If the user suggests changes, call this again with the updated message.",
      inputSchema: {
        type: "object",
        properties: {
          commit_point_id: { type: "string", description: "Required id of the commit_point to draft for." },
          message: { type: "string", description: "Required drafted commit message. Conventional-Commits style recommended." },
        },
        required: ["commit_point_id", "message"],
      },
      handler: (args: { commit_point_id: string; message: string }) => {
        const updated = commitPointStore.propose(args.commit_point_id, args.message);
        return { ok: true, commitPoint: updated, awaiting: "user-approval" };
      },
    },
    {
      name: "newde__commit",
      description:
        "Run the git commit for an active commit point. Only call this AFTER the user has explicitly approved your drafted message in chat. The `message` you pass here is the final message that will be committed (usually the same as your most recent propose_commit draft). Throws on git failure; read the error, fix the underlying issue, and call again.",
      inputSchema: {
        type: "object",
        properties: {
          commit_point_id: { type: "string", description: "Required id of the commit_point to execute." },
          message: { type: "string", description: "Required final commit message." },
        },
        required: ["commit_point_id", "message"],
      },
      handler: (args: { commit_point_id: string; message: string }) => {
        const updated = executeCommit(args.commit_point_id, args.message);
        return { ok: true, commitPoint: updated, commitSha: updated.commit_sha };
      },
    },
    {
      name: "newde__list_commit_points",
      description: "List commit points for a batch, ordered by their position in the work queue.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: { type: "string", description: "Optional. Server infers the owning stream from batchId when omitted; only passed explicitly for the no-batchId form of get_batch_context." },
          batchId: { type: "string", description: "Required batch id." },
        },
        required: ["batchId"],
      },
      handler: (args: { streamId?: string; batchId: string }) => {
        resolveBatchAndStream(args);
        return { commitPoints: commitPointStore.listForBatch(args.batchId) };
      },
    },
  ];
}
