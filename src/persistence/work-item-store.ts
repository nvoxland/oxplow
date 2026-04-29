import type { Logger } from "../core/logger.js";
import { createId } from "../core/ids.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";

export type WorkItemKind = "epic" | "task" | "subtask" | "bug" | "note";
export type WorkItemStatus = "ready" | "in_progress" | "blocked" | "done" | "canceled" | "archived";
export type WorkItemPriority = "low" | "medium" | "high" | "urgent";
export type WorkItemLinkType =
  | "blocks"
  | "relates_to"
  | "discovered_from"
  | "duplicates"
  | "supersedes"
  | "replies_to";
export type WorkItemActorKind = "user" | "agent" | "system";
/** Semantic origin of a work-item row — distinct from `created_by` (the
 *  writer). Narrowed to user/agent after auto-file was removed in v29+;
 *  legacy `agent-auto` rows exist on pre-v29 DBs but v29 cancels any
 *  still-in_progress ones. The read path tolerates the legacy string by
 *  mapping it to null so older rows still load. */
export type WorkItemAuthor = "user" | "agent";

const WORK_ITEM_AUTHORS: ReadonlySet<WorkItemAuthor> = new Set([
  "user", "agent",
]);

const WORK_ITEM_KINDS: ReadonlySet<WorkItemKind> = new Set([
  "epic", "task", "subtask", "bug", "note",
]);
const WORK_ITEM_STATUSES: ReadonlySet<WorkItemStatus> = new Set([
  "ready", "in_progress", "blocked", "done", "canceled", "archived",
]);
const WORK_ITEM_PRIORITIES: ReadonlySet<WorkItemPriority> = new Set([
  "low", "medium", "high", "urgent",
]);
const WORK_ITEM_LINK_TYPES: ReadonlySet<WorkItemLinkType> = new Set([
  "blocks", "relates_to", "discovered_from", "duplicates", "supersedes", "replies_to",
]);
const WORK_ITEM_ACTOR_KINDS: ReadonlySet<WorkItemActorKind> = new Set([
  "user", "agent", "system",
]);

// External input caps — keep noisy payloads from corrupting the store or
// blowing up memory when reached from MCP/IPC. Not internal limits.
const TITLE_MAX_LEN = 500;
const DESCRIPTION_MAX_LEN = 20_000;
const ACCEPTANCE_CRITERIA_MAX_LEN = 20_000;
const NOTE_MAX_LEN = 20_000;

export const BACKLOG_SCOPE = "__backlog__";

export interface WorkItem {
  id: string;
  thread_id: string | null;
  parent_id: string | null;
  kind: WorkItemKind;
  title: string;
  description: string;
  acceptance_criteria: string | null;
  status: WorkItemStatus;
  priority: WorkItemPriority;
  sort_index: number;
  created_by: WorkItemActorKind;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
  deleted_at: string | null;
  note_count: number;
  /** Semantic origin — see `WorkItemAuthor`. Null for legacy rows. */
  author: WorkItemAuthor | null;
  /** Free-text grooming bucket used by the Backlog page's group-by.
   *  Persists across promote/demote; null when unset. */
  category: string | null;
  /** Comma-separated tags used by the Backlog page filter chips.
   *  Persists across promote/demote; null when unset. */
  tags: string | null;
}

export interface WorkNote {
  id: string;
  /** Non-null when this note is attached to an individual work item. Mutually
   *  exclusive with `thread_id` (enforced by a CHECK at the DB layer in
   *  migration v25). */
  work_item_id: string | null;
  /** Non-null when this is a thread-scoped note (not attached to any work
   *  item) — typically findings from `oxplow__delegate_query` Explore-subagent
   *  runs landed here via `oxplow__record_query_finding`. */
  thread_id: string | null;
  body: string;
  author: string;
  created_at: string;
}

export interface WorkItemLink {
  id: string;
  thread_id: string;
  from_item_id: string;
  to_item_id: string;
  link_type: WorkItemLinkType;
  created_at: string;
}

export interface WorkItemDetail {
  item: WorkItem;
  outgoing: WorkItemLink[];
  incoming: WorkItemLink[];
  recentEvents: WorkItemEvent[];
}

export interface WorkItemWithLinks {
  item: WorkItem;
  outgoing: WorkItemLink[];
  incoming: WorkItemLink[];
}

export interface EpicWorkUnit {
  mode: "epic";
  epic: WorkItem;
  children: WorkItemWithLinks[];
}

export interface StandaloneWorkUnit {
  mode: "standalone";
  items: WorkItemWithLinks[];
}

export interface EmptyWorkUnit {
  mode: "empty";
}

export type ReadWorkOptionsResult = EpicWorkUnit | StandaloneWorkUnit | EmptyWorkUnit;

export interface WorkItemEvent {
  id: string;
  thread_id: string | null;
  item_id: string | null;
  event_type: string;
  actor_kind: WorkItemActorKind;
  actor_id: string;
  payload_json: string;
  created_at: string;
}

export type WorkItemChangeKind = "created" | "updated" | "note" | "linked" | "deleted" | "reordered" | "moved";

export interface WorkItemChange {
  threadId: string;
  kind: WorkItemChangeKind;
  itemId: string | null;
  /** Only populated for `updated` events when the status changed. */
  previousStatus?: WorkItemStatus;
  /** Only populated for `updated` events when the status changed. */
  nextStatus?: WorkItemStatus;
  /**
   * Optional list of repo-relative paths the agent declares it touched
   * during this effort. Only forwarded when the status transitions to
   * `done`; consumed by the effort-close path to populate
   * `work_item_effort_file`. Server dedups and caps at 100 paths.
   */
  touchedFiles?: string[];
}

export interface ThreadFollowup {
  id: string;
  note: string;
  createdAt: string;
}

export interface ThreadWorkState {
  threadId: string;
  waiting: WorkItem[];
  inProgress: WorkItem[];
  done: WorkItem[];
  epics: WorkItem[];
  items: WorkItem[];
  /** Transient agent follow-up reminders (in-memory, lost on restart).
   *  Always present; empty array when none. Surfaced at the top of the
   *  To Do section in the Work panel. */
  followups: ThreadFollowup[];
}

interface CreateWorkItemInput {
  threadId: string;
  parentId?: string | null;
  kind: WorkItemKind;
  title: string;
  description?: string;
  acceptanceCriteria?: string | null;
  status?: WorkItemStatus;
  priority?: WorkItemPriority;
  createdBy: WorkItemActorKind;
  actorId: string;
  /** Optional semantic origin. See WorkItemAuthor. */
  author?: WorkItemAuthor | null;
  /** Optional grooming category — see WorkItem.category. */
  category?: string | null;
  /** Optional comma-separated tags — see WorkItem.tags. */
  tags?: string | null;
}

export interface FileEpicWithChildrenInput {
  threadId: string;
  epic: {
    title: string;
    description?: string;
    acceptanceCriteria?: string | null;
    priority?: WorkItemPriority;
  };
  children: Array<{
    title: string;
    description?: string;
    acceptanceCriteria?: string | null;
    priority?: WorkItemPriority;
    kind?: WorkItemKind;
  }>;
  createdBy: WorkItemActorKind;
  actorId: string;
}

export interface CompleteTaskInput {
  threadId: string;
  itemId: string;
  note: string;
  /** Defaults to `done`. Only `done` and `blocked` are valid finishers. */
  status?: "done" | "blocked";
  /** Optional list of repo-relative paths the agent touched during this
   *  effort. Forwarded to the underlying status transition so the runtime
   *  can insert `work_item_effort_file` rows at effort-close. This is the
   *  ONE-shot attribution point for close-via-complete_task — if omitted,
   *  the effort closes with zero attributed files and the Local History
   *  panel falls back to "assume all" for this item. */
  touchedFiles?: string[];
  actorKind: WorkItemActorKind;
  actorId: string;
}

interface UpdateWorkItemInput {
  threadId: string;
  itemId: string;
  title?: string;
  description?: string;
  acceptanceCriteria?: string | null;
  status?: WorkItemStatus;
  priority?: WorkItemPriority;
  parentId?: string | null;
  /** Optional author change. Used by the auto-file → explicit-adoption flow
   *  to flip 'agent-auto' → 'agent' in place. */
  author?: WorkItemAuthor | null;
  /** Optional grooming category. Pass null to clear, omit to keep. */
  category?: string | null;
  /** Optional comma-separated tags. Pass null to clear, omit to keep. */
  tags?: string | null;
  actorKind: WorkItemActorKind;
  actorId: string;
  /**
   * Optional list of repo-relative paths the agent touched during this
   * effort. Relevant only when transitioning to `done`; ignored
   * otherwise. Passed through to the `WorkItemChange` event so the
   * runtime can insert `work_item_effort_file` rows at effort-close.
   */
  touchedFiles?: string[];
}

export class WorkItemStore {
  private readonly stateDb;
  private readonly emitter: StoreEmitter<WorkItemChange>;

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.emitter = new StoreEmitter("work item change", logger);
  }

  subscribe(listener: (change: WorkItemChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  private emitChange(change: WorkItemChange): void {
    this.emitter.emit(change);
  }

  getState(threadId: string): ThreadWorkState {
    const items = this.listItems(threadId);
    return {
      threadId,
      waiting: items.filter((item) => item.status === "ready" || item.status === "blocked"),
      inProgress: items.filter((item) => item.status === "in_progress"),
      done: items.filter((item) => item.status === "done" || item.status === "canceled" || item.status === "archived"),
      epics: items.filter((item) => item.kind === "epic"),
      items,
      // Followups are layered in by the work-item API wrapper from a
      // transient in-memory store. The persistence layer doesn't own
      // them, so default to empty here and let the wrapper overwrite.
      followups: [],
    };
  }

  listItems(threadId: string): WorkItem[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT work_items.*,
                (SELECT COUNT(*) FROM work_note WHERE work_note.work_item_id = work_items.id) AS note_count
         FROM work_items
         WHERE thread_id = ? AND deleted_at IS NULL
         ORDER BY sort_index, created_at, id`,
        threadId,
      )
      .map(toWorkItem);
  }

  /** Find any in_progress work item in a thread (regardless of author).
   *  Used by the auto-file guard to decide whether to spawn a new
   *  agent-auto row — if ANY in_progress item already exists in the
   *  thread (agent-filed, user-filed, already-adopted, reopened, etc.),
   *  the current turn's edits attribute to it and a duplicate auto-row
   *  would be a zombie. See wi-e79eaffd7cf0. Returns the oldest
   *  in_progress row so callers see a deterministic pick when two exist. */
  findOpenItemForThread(threadId: string): WorkItem | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT work_items.*,
              (SELECT COUNT(*) FROM work_note WHERE work_note.work_item_id = work_items.id) AS note_count
       FROM work_items
       WHERE thread_id = ?
         AND status = 'in_progress'
         AND deleted_at IS NULL
       ORDER BY created_at, id
       LIMIT 1`,
      threadId,
    );
    return row ? toWorkItem(row) : null;
  }

  /**
   * Thread-agnostic lookup of `{id, title, status, thread_id}` for a set
   * of item ids. Skips deleted rows. Used by the recent-activity surfaces
   * that need to render usage events as titles regardless of which thread
   * the item currently belongs to (or whether it's now in the backlog).
   */
  getSummariesByIds(ids: string[]): Array<{ id: string; title: string; status: WorkItemStatus; thread_id: string | null }> {
    if (ids.length === 0) return [];
    const placeholders = ids.map(() => "?").join(",");
    const rows = this.stateDb.all<{ id: string; title: string; status: string; thread_id: string | null }>(
      `SELECT id, title, status, thread_id
         FROM work_items
        WHERE id IN (${placeholders}) AND deleted_at IS NULL`,
      ...ids,
    );
    return rows.map((r) => ({
      id: r.id,
      title: r.title,
      status: requireWorkItemStatus(r.status),
      thread_id: r.thread_id,
    }));
  }

  getItem(threadId: string, itemId: string): WorkItem | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT work_items.*,
              (SELECT COUNT(*) FROM work_note WHERE work_note.work_item_id = work_items.id) AS note_count
       FROM work_items
       WHERE thread_id = ? AND id = ? AND deleted_at IS NULL
       LIMIT 1`,
      threadId,
      itemId,
    );
    return row ? toWorkItem(row) : null;
  }

  createItem(input: CreateWorkItemInput): WorkItem {
    const title = requireTitle(input.title);
    const description = clampDescription(input.description);
    const acceptance = clampAcceptanceCriteria(input.acceptanceCriteria);
    const kind = requireWorkItemKind(input.kind);
    const status = input.status ? requireWorkItemStatus(input.status) : "ready";
    const priority = input.priority ? requireWorkItemPriority(input.priority) : "medium";
    const createdBy = requireWorkItemActorKind(input.createdBy);
    const parentId = input.parentId ?? null;
    const author = input.author === undefined ? null : requireOptionalWorkItemAuthor(input.author);
    const category = clampCategory(input.category);
    const tags = clampTags(input.tags);
    const now = new Date().toISOString();
    const id = createId("wi");

    const item: WorkItem = {
      id,
      thread_id: input.threadId,
      parent_id: parentId,
      kind,
      title,
      description,
      acceptance_criteria: acceptance,
      status,
      priority,
      sort_index: 0, // filled in by the INSERT subquery
      created_by: createdBy,
      created_at: now,
      updated_at: now,
      completed_at: status === "done" ? now : null,
      deleted_at: null,
      note_count: 0,
      author,
      category,
      tags,
    };

    this.stateDb.transaction(() => {
      this.insertItemRow(item, input.actorId);
    });
    this.logger?.info("created work item", {
      threadId: item.thread_id,
      itemId: item.id,
      kind: item.kind,
      status: item.status,
    });
    this.emitChange({ threadId: item.thread_id ?? BACKLOG_SCOPE, kind: "created", itemId: item.id });
    return item;
  }

  /** Insert a fully-built item row + record the "created" event. Assumes the
   *  caller has opened (or is outside) a transaction — does NOT open its own.
   *  Mutates `item.sort_index` to the DB-assigned value. */
  private insertItemRow(item: WorkItem, actorId: string): void {
    if (item.parent_id) this.requireItemInThread(item.thread_id!, item.parent_id, "parent");
    const parentClause = item.parent_id ? "parent_id = ?" : "parent_id IS NULL";
    const sortParams: [string, ...(string[])] = item.parent_id
      ? [item.thread_id!, item.parent_id]
      : [item.thread_id!];
    this.stateDb.run(
      `INSERT INTO work_items (
        id, thread_id, parent_id, kind, title, description, acceptance_criteria, status, priority,
        sort_index, created_by, created_at, updated_at, completed_at, author, category, tags
      ) VALUES (
        ?, ?, ?, ?, ?, ?, ?, ?, ?,
        (SELECT COALESCE(MAX(sort_index), -1) + 1 FROM work_items WHERE thread_id = ? AND ${parentClause}),
        ?, ?, ?, ?, ?, ?, ?
      )`,
      item.id,
      item.thread_id,
      item.parent_id,
      item.kind,
      item.title,
      item.description,
      item.acceptance_criteria,
      item.status,
      item.priority,
      ...sortParams,
      item.created_by,
      item.created_at,
      item.updated_at,
      item.completed_at,
      item.author,
      item.category,
      item.tags,
    );
    const stored = this.getItem(item.thread_id!, item.id);
    if (!stored) throw new Error("work item was not persisted");
    item.sort_index = stored.sort_index;
    this.recordEvent({
      threadId: item.thread_id!,
      itemId: item.id,
      eventType: "created",
      actorKind: item.created_by,
      actorId,
      payload: item,
    });
  }

  updateItem(input: UpdateWorkItemInput): WorkItem {
    const existing = this.getItem(input.threadId, input.itemId);
    if (!existing) throw new Error(`unknown work item: ${input.itemId}`);
    const actorKind = requireWorkItemActorKind(input.actorKind);
    const now = new Date().toISOString();
    const nextParentId = input.parentId !== undefined ? input.parentId : existing.parent_id;
    const nextStatus = input.status ? requireWorkItemStatus(input.status) : existing.status;
    // Transition guard: block jumping to in_progress from canceled/archived
    // (those are explicit "abandoned" states the user must re-ready first).
    // `done → in_progress` is allowed: it's the redo/reopen path when the
    // user pushes back on shipped work. `blocked → in_progress` is also
    // allowed (deliberate unblock gesture; see wi-6285706789c5).
    if (nextStatus === "in_progress" && existing.status !== nextStatus) {
      if (existing.status === "canceled" || existing.status === "archived") {
        throw new Error(
          `cannot transition \`${existing.status}\` → \`in_progress\` directly; move to \`ready\` first`,
        );
      }
    }
    const nextPriority = input.priority ? requireWorkItemPriority(input.priority) : existing.priority;
    const nextTitle = input.title !== undefined ? requireTitle(input.title) : existing.title;
    const nextDescription = input.description !== undefined ? clampDescription(input.description) : existing.description;
    const nextAcceptance = input.acceptanceCriteria !== undefined
      ? clampAcceptanceCriteria(input.acceptanceCriteria)
      : existing.acceptance_criteria;

    const nextAuthor: WorkItem["author"] = input.author !== undefined
      ? (input.author === null ? null : requireOptionalWorkItemAuthor(input.author))
      : existing.author;
    const nextCategory: WorkItem["category"] = input.category !== undefined
      ? clampCategory(input.category)
      : existing.category;
    const nextTags: WorkItem["tags"] = input.tags !== undefined
      ? clampTags(input.tags)
      : existing.tags;
    const updated: WorkItem = {
      ...existing,
      parent_id: nextParentId,
      title: nextTitle,
      description: nextDescription,
      acceptance_criteria: nextAcceptance,
      status: nextStatus,
      priority: nextPriority,
      updated_at: now,
      completed_at: nextStatus === "done"
        ? (existing.completed_at ?? now)
        : existing.status === "done"
          ? null
          : existing.completed_at,
      author: nextAuthor,
      category: nextCategory,
      tags: nextTags,
    };
    if (nextParentId && nextParentId === input.itemId) {
      throw new Error("work item cannot be its own parent");
    }

    // When an item transitions into "done" (from any non-done status), bump
    // its sort_index to MAX+1 within the thread so the Done section (which
    // renders descending by sort_index) places it at the top. Canceled /
    // archived also render in the Done visual bucket; treat them the same.
    const nowDoneLike = nextStatus === "done" || nextStatus === "canceled" || nextStatus === "archived";
    const wasDoneLike = existing.status === "done" || existing.status === "canceled" || existing.status === "archived";
    const bumpSortIndex = nowDoneLike && !wasDoneLike;

    this.stateDb.transaction(() => {
      if (nextParentId && nextParentId !== existing.parent_id) {
        this.requireItemInThread(input.threadId, nextParentId, "parent");
      }
      if (bumpSortIndex) {
        const row = this.stateDb.get<{ next_index: number }>(
          `SELECT COALESCE(MAX(sort_index), -1) + 1 AS next_index
           FROM work_items WHERE thread_id = ?`,
          updated.thread_id,
        );
        updated.sort_index = row?.next_index ?? 0;
      }
      this.stateDb.run(
        `UPDATE work_items
         SET parent_id = ?, title = ?, description = ?, acceptance_criteria = ?, status = ?, priority = ?, sort_index = ?, updated_at = ?, completed_at = ?, author = ?, category = ?, tags = ?
         WHERE thread_id = ? AND id = ?`,
        updated.parent_id,
        updated.title,
        updated.description,
        updated.acceptance_criteria,
        updated.status,
        updated.priority,
        updated.sort_index,
        updated.updated_at,
        updated.completed_at,
        updated.author,
        updated.category,
        updated.tags,
        updated.thread_id,
        updated.id,
      );
      this.recordEvent({
        threadId: input.threadId,
        itemId: input.itemId,
        eventType: "updated",
        actorKind,
        actorId: input.actorId,
        payload: snapshotWorkItem(existing, updated),
      });
    });
    const statusChanged = existing.status !== nextStatus;
    this.emitChange({
      threadId: input.threadId,
      kind: "updated",
      itemId: input.itemId,
      previousStatus: statusChanged ? existing.status : undefined,
      nextStatus: statusChanged ? nextStatus : undefined,
      touchedFiles: statusChanged && (nextStatus === "done" || nextStatus === "blocked") ? input.touchedFiles : undefined,
    });
    return updated;
  }

  addNote(
    threadId: string,
    itemId: string | null,
    note: string,
    actorKind: WorkItemActorKind,
    actorId: string,
  ): void {
    const trimmed = note.trim();
    if (!trimmed) throw new Error("note is required");
    if (trimmed.length > NOTE_MAX_LEN) {
      throw new Error(`note too long: max ${NOTE_MAX_LEN} chars`);
    }
    const kind = requireWorkItemActorKind(actorKind);
    this.stateDb.transaction(() => {
      if (itemId) this.requireItemInThread(threadId, itemId, "note target");
      this.recordEvent({
        threadId,
        itemId,
        eventType: "note",
        actorKind: kind,
        actorId,
        payload: { note: trimmed },
      });
      // Also land the note in the structured `work_note` table so the UI's
      // getWorkNotes reader surfaces it (the historical event stream alone
      // isn't queried by the Work panel's note list).
      if (itemId) {
        this.stateDb.run(
          `INSERT INTO work_note (id, work_item_id, thread_id, body, author, created_at)
           VALUES (?, ?, NULL, ?, ?, ?)`,
          createId("note"),
          itemId,
          trimmed,
          String(actorId || kind),
          new Date().toISOString(),
        );
      }
    });
    this.emitChange({ threadId, kind: "note", itemId });
  }

  /** Atomically create an epic plus N child items in a single transaction.
   *  Rejects empty children — the whole point of this entry point is to
   *  prevent the epic-without-children footgun. Children default to
   *  `kind: "task"` if unspecified. All rows share the same transaction: a
   *  validation failure on the Nth child rolls back the epic too, so no
   *  partial-creation state is possible. */
  fileEpicWithChildren(input: FileEpicWithChildrenInput): { epicId: string; childIds: string[] } {
    if (!Array.isArray(input.children) || input.children.length === 0) {
      throw new Error("file_epic_with_children requires at least one child — an epic without children is a bug");
    }
    const now = new Date().toISOString();
    const createdBy = requireWorkItemActorKind(input.createdBy);
    const buildItem = (
      opts: {
        kind: WorkItemKind;
        title: string;
        description?: string;
        acceptanceCriteria?: string | null;
        priority?: WorkItemPriority;
        parentId: string | null;
      },
    ): WorkItem => ({
      id: createId("wi"),
      thread_id: input.threadId,
      parent_id: opts.parentId,
      kind: requireWorkItemKind(opts.kind),
      title: requireTitle(opts.title),
      description: clampDescription(opts.description),
      acceptance_criteria: clampAcceptanceCriteria(opts.acceptanceCriteria),
      status: "ready",
      priority: opts.priority ? requireWorkItemPriority(opts.priority) : "medium",
      sort_index: 0,
      created_by: createdBy,
      created_at: now,
      updated_at: now,
      completed_at: null,
      deleted_at: null,
      note_count: 0,
      author: null,
      category: null,
      tags: null,
    });

    // All rows (epic + children) live in ONE transaction so a validation
    // failure on any child rolls back the epic too.
    let epicId: string;
    const children: WorkItem[] = [];
    this.stateDb.transaction(() => {
      const epic = buildItem({
        kind: "epic",
        title: input.epic.title,
        description: input.epic.description,
        acceptanceCriteria: input.epic.acceptanceCriteria,
        priority: input.epic.priority,
        parentId: null,
      });
      this.insertItemRow(epic, input.actorId);
      epicId = epic.id;
      for (const childInput of input.children) {
        const child = buildItem({
          kind: childInput.kind ?? "task",
          title: childInput.title,
          description: childInput.description,
          acceptanceCriteria: childInput.acceptanceCriteria,
          priority: childInput.priority,
          parentId: epicId,
        });
        this.insertItemRow(child, input.actorId);
        children.push(child);
      }
    });
    this.emitChange({ threadId: input.threadId, kind: "created", itemId: epicId! });
    for (const child of children) {
      this.emitChange({ threadId: input.threadId, kind: "created", itemId: child.id });
    }
    return { epicId: epicId!, childIds: children.map((c) => c.id) };
  }

  /** Validate + apply the closing status transition. The caller's
   *  `note` text is NOT appended to the work-item history anymore — it
   *  belongs on the effort row that just closed (written by the
   *  caller after this returns; see `mcp-tools.ts`'s
   *  `oxplow__complete_task` handler). `status` defaults to `done`.
   *  Rejects if the current status is already terminal. */
  completeTask(input: CompleteTaskInput): WorkItem {
    const status = input.status ?? "done";
    if (status !== "done" && status !== "blocked") {
      throw new Error(
        `complete_task: status must be 'done' or 'blocked'`,
      );
    }
    const existing = this.getItem(input.threadId, input.itemId);
    if (!existing) throw new Error(`unknown work item: ${input.itemId}`);
    if (existing.status === "done" || existing.status === "canceled" || existing.status === "archived") {
      throw new Error(
        `complete_task: item ${input.itemId} is already in terminal status '${existing.status}' — use update_work_item to change it`,
      );
    }
    const updated = this.updateItem({
      threadId: input.threadId,
      itemId: input.itemId,
      status,
      touchedFiles: input.touchedFiles,
      actorKind: input.actorKind,
      actorId: input.actorId,
    });
    return updated;
  }

  deleteItem(threadId: string, itemId: string, actorKind: WorkItemActorKind, actorId: string): void {
    const kind = requireWorkItemActorKind(actorKind);
    const now = new Date().toISOString();
    let deleted = false;
    this.stateDb.transaction(() => {
      const existing = this.getItem(threadId, itemId);
      if (!existing) return;
      this.stateDb.run(
        `UPDATE work_items SET deleted_at = ?, updated_at = ? WHERE thread_id = ? AND id = ?`,
        now,
        now,
        threadId,
        itemId,
      );
      this.recordEvent({
        threadId,
        itemId,
        eventType: "deleted",
        actorKind: kind,
        actorId,
        payload: { before: existing },
      });
      deleted = true;
    });
    if (deleted) {
      this.logger?.info("deleted work item", { threadId, itemId });
      this.emitChange({ threadId, kind: "deleted", itemId });
    }
  }

  /** Explicit sort_index writes — used by the thread-queue reorder. */
  setItemSortIndexes(threadId: string, entries: Array<{ id: string; sortIndex: number }>): void {
    if (entries.length === 0) return;
    const now = new Date().toISOString();
    this.stateDb.transaction(() => {
      for (const entry of entries) {
        this.stateDb.run(
          `UPDATE work_items SET sort_index = ?, updated_at = ? WHERE thread_id = ? AND id = ?`,
          entry.sortIndex, now, threadId, entry.id,
        );
      }
    });
    this.emitChange({ threadId, kind: "reordered", itemId: null });
  }

  reorderItems(
    threadId: string,
    orderedItemIds: string[],
    actorKind: WorkItemActorKind,
    actorId: string,
  ): void {
    if (orderedItemIds.length === 0) return;
    if (new Set(orderedItemIds).size !== orderedItemIds.length) {
      throw new Error("reorder request contained duplicate ids");
    }
    const kind = requireWorkItemActorKind(actorKind);
    const now = new Date().toISOString();
    this.stateDb.transaction(() => {
      const rows = this.stateDb.all<{ id: string; parent_id: string | null }>(
        `SELECT id, parent_id FROM work_items
         WHERE thread_id = ? AND id IN (${orderedItemIds.map(() => "?").join(",")}) AND deleted_at IS NULL`,
        threadId,
        ...orderedItemIds,
      );
      if (rows.length !== orderedItemIds.length) {
        throw new Error("reorder request referenced unknown or deleted work items");
      }
      const parent = rows[0]!.parent_id;
      if (rows.some((row) => row.parent_id !== parent)) {
        throw new Error("reorder request mixed items with different parents");
      }
      for (let index = 0; index < orderedItemIds.length; index++) {
        this.stateDb.run(
          `UPDATE work_items SET sort_index = ?, updated_at = ? WHERE thread_id = ? AND id = ?`,
          index,
          now,
          threadId,
          orderedItemIds[index]!,
        );
      }
      this.recordEvent({
        threadId,
        itemId: null,
        eventType: "reordered",
        actorKind: kind,
        actorId,
        payload: { orderedItemIds },
      });
    });
    this.emitChange({ threadId, kind: "reordered", itemId: null });
  }

  moveItemToScope(
    fromScope: string | null,
    itemId: string,
    toScope: string | null,
    actorKind: WorkItemActorKind,
    actorId: string,
  ): void {
    if (fromScope === toScope) return;
    const kind = requireWorkItemActorKind(actorKind);
    const now = new Date().toISOString();
    this.stateDb.transaction(() => {
      const root = this.getItemInScope(fromScope, itemId);
      if (!root) throw new Error("work item not found in source scope");
      if (toScope !== null) {
        const targetExists = this.stateDb.get<{ id: string }>(
          `SELECT id FROM threads WHERE id = ? LIMIT 1`,
          toScope,
        );
        if (!targetExists) throw new Error("target thread not found");
      }

      const descendantIds: string[] = [];
      const queue: string[] = [itemId];
      while (queue.length > 0) {
        const current = queue.shift()!;
        const children = fromScope === null
          ? this.stateDb.all<{ id: string }>(
              `SELECT id FROM work_items WHERE thread_id IS NULL AND parent_id = ? AND deleted_at IS NULL`,
              current,
            )
          : this.stateDb.all<{ id: string }>(
              `SELECT id FROM work_items WHERE thread_id = ? AND parent_id = ? AND deleted_at IS NULL`,
              fromScope,
              current,
            );
        for (const child of children) {
          descendantIds.push(child.id);
          queue.push(child.id);
        }
      }

      const nextSortIndex = (toScope === null
        ? this.stateDb.get<{ next_index: number }>(
            `SELECT COALESCE(MAX(sort_index), -1) + 1 AS next_index
             FROM work_items WHERE thread_id IS NULL AND parent_id IS NULL AND deleted_at IS NULL`,
          )
        : this.stateDb.get<{ next_index: number }>(
            `SELECT COALESCE(MAX(sort_index), -1) + 1 AS next_index
             FROM work_items WHERE thread_id = ? AND parent_id IS NULL AND deleted_at IS NULL`,
            toScope,
          ))?.next_index ?? 0;

      if (fromScope === null) {
        this.stateDb.run(
          `UPDATE work_items SET thread_id = ?, parent_id = NULL, sort_index = ?, updated_at = ?
           WHERE thread_id IS NULL AND id = ?`,
          toScope,
          nextSortIndex,
          now,
          itemId,
        );
        for (const descendantId of descendantIds) {
          this.stateDb.run(
            `UPDATE work_items SET thread_id = ?, updated_at = ? WHERE thread_id IS NULL AND id = ?`,
            toScope,
            now,
            descendantId,
          );
        }
      } else {
        this.stateDb.run(
          `UPDATE work_items SET thread_id = ?, parent_id = NULL, sort_index = ?, updated_at = ?
           WHERE thread_id = ? AND id = ?`,
          toScope,
          nextSortIndex,
          now,
          fromScope,
          itemId,
        );
        for (const descendantId of descendantIds) {
          this.stateDb.run(
            `UPDATE work_items SET thread_id = ?, updated_at = ? WHERE thread_id = ? AND id = ?`,
            toScope,
            now,
            fromScope,
            descendantId,
          );
        }
      }

      this.recordEvent({
        threadId: fromScope,
        itemId,
        eventType: "moved_out",
        actorKind: kind,
        actorId,
        payload: { toScope, movedItemIds: [itemId, ...descendantIds] },
      });
      this.recordEvent({
        threadId: toScope,
        itemId,
        eventType: "moved_in",
        actorKind: kind,
        actorId,
        payload: { fromScope, movedItemIds: [itemId, ...descendantIds] },
      });
    });
    this.logger?.info("moved work item between scopes", { fromScope, toScope, itemId });
    this.emitChange({ threadId: fromScope ?? BACKLOG_SCOPE, kind: "moved", itemId });
    this.emitChange({ threadId: toScope ?? BACKLOG_SCOPE, kind: "moved", itemId });
  }

  /** Backwards-compatible alias for thread-to-thread moves. */
  moveItemToThread(
    fromThreadId: string,
    itemId: string,
    toThreadId: string,
    actorKind: WorkItemActorKind,
    actorId: string,
  ): void {
    this.moveItemToScope(fromThreadId, itemId, toThreadId, actorKind, actorId);
  }

  listBacklog(): WorkItem[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT work_items.*,
                (SELECT COUNT(*) FROM work_note WHERE work_note.work_item_id = work_items.id) AS note_count
         FROM work_items
         WHERE thread_id IS NULL AND deleted_at IS NULL
         ORDER BY sort_index, created_at, id`,
      )
      .map(toWorkItem);
  }

  getBacklogItem(itemId: string): WorkItem | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT work_items.*,
              (SELECT COUNT(*) FROM work_note WHERE work_note.work_item_id = work_items.id) AS note_count
       FROM work_items
       WHERE thread_id IS NULL AND id = ? AND deleted_at IS NULL
       LIMIT 1`,
      itemId,
    );
    return row ? toWorkItem(row) : null;
  }

  getWorkNotes(itemId: string): WorkNote[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM work_note WHERE work_item_id = ? ORDER BY created_at ASC`,
        itemId,
      )
      .map(toWorkNote);
  }

  /**
   * Copy the last `limit` notes of `itemId` (ordered by `created_at DESC`,
   * then re-inserted chronologically so the new rows land in original
   * order) as fresh rows attached to the same item id. Used by
   * `fork_thread` to carry per-item note history into the new thread —
   * the moved item's history continues to be readable after the fork.
   *
   * - Returns the number of rows inserted.
   * - Source rows are never modified.
   * - Fresh ids and fresh `created_at` timestamps; body+author preserved.
   * - No-op and returns 0 when the item has no notes or when `limit <= 0`.
   */
  copyLastItemNotes(itemId: string, limit: number): number {
    const cap = Math.max(0, Math.floor(Number(limit) || 0));
    if (cap === 0) return 0;
    const recent = this.stateDb.all<Record<string, unknown>>(
      `SELECT body, author FROM work_note
       WHERE work_item_id = ?
       ORDER BY created_at DESC, id DESC
       LIMIT ?`,
      itemId,
      cap,
    );
    if (recent.length === 0) return 0;
    // Re-insert oldest-first so the copies' created_at ordering matches
    // the originals' chronology.
    const ordered = recent.slice().reverse();
    this.stateDb.transaction(() => {
      for (const row of ordered) {
        const id = createId("note");
        const now = new Date().toISOString();
        this.stateDb.run(
          `INSERT INTO work_note (id, work_item_id, thread_id, body, author, created_at)
           VALUES (?, ?, NULL, ?, ?, ?)`,
          id,
          itemId,
          String(row.body ?? ""),
          String(row.author ?? "agent"),
          now,
        );
      }
    });
    return ordered.length;
  }

  /** Insert a thread-scoped note row (no work item). Returns the inserted
   *  row's id. Used by `oxplow__delegate_query` to pre-allocate a landing
   *  slot for Explore-subagent findings, and by `record_query_finding` to
   *  fill the body in once the subagent returns. Pass `body = ""` at
   *  allocation time and update it later via `updateThreadNoteBody`. */
  addThreadNote(threadId: string, body: string, author: string): string {
    const trimmedAuthor = String(author ?? "").trim();
    if (!trimmedAuthor) throw new Error("thread note author is required");
    if (body != null && body.length > NOTE_MAX_LEN) {
      throw new Error(`note too long: max ${NOTE_MAX_LEN} chars`);
    }
    const id = createId("note");
    const now = new Date().toISOString();
    this.stateDb.run(
      `INSERT INTO work_note (id, work_item_id, thread_id, body, author, created_at)
       VALUES (?, NULL, ?, ?, ?, ?)`,
      id,
      threadId,
      String(body ?? ""),
      trimmedAuthor,
      now,
    );
    return id;
  }

  /** Overwrite the `body` of an existing thread-scoped note. Used by
   *  `oxplow__record_query_finding` to fill in a pre-allocated row once the
   *  Explore subagent returns its finding. Throws if the note doesn't
   *  exist or isn't thread-scoped (belongs to a work item instead). */
  updateThreadNoteBody(noteId: string, body: string): void {
    if (body == null) throw new Error("body is required");
    if (body.length > NOTE_MAX_LEN) {
      throw new Error(`note too long: max ${NOTE_MAX_LEN} chars`);
    }
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT thread_id FROM work_note WHERE id = ?`,
      noteId,
    );
    if (!row) throw new Error(`unknown thread note: ${noteId}`);
    if (row.thread_id == null) {
      throw new Error(`note ${noteId} is item-scoped, not thread-scoped`);
    }
    this.stateDb.run(
      `UPDATE work_note SET body = ? WHERE id = ?`,
      body,
      noteId,
    );
  }

  /** Return up to `limit` most-recent thread-scoped notes (reverse chronological). */
  listThreadNotes(threadId: string, limit = 5): WorkNote[] {
    const cap = Math.max(1, Math.min(Number(limit) || 5, 100));
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM work_note
         WHERE thread_id = ?
         ORDER BY created_at DESC, id DESC
         LIMIT ?`,
        threadId,
        cap,
      )
      .map(toWorkNote);
  }

  private getItemInScope(scope: string | null, itemId: string): WorkItem | null {
    if (scope === null) return this.getBacklogItem(itemId);
    return this.getItem(scope, itemId);
  }

  createBacklogItem(input: {
    kind: WorkItemKind;
    title: string;
    description?: string;
    acceptanceCriteria?: string | null;
    status?: WorkItemStatus;
    priority?: WorkItemPriority;
    category?: string | null;
    tags?: string | null;
    createdBy: WorkItemActorKind;
    actorId: string;
  }): WorkItem {
    const title = requireTitle(input.title);
    const description = clampDescription(input.description);
    const acceptance = clampAcceptanceCriteria(input.acceptanceCriteria);
    const kind = requireWorkItemKind(input.kind);
    const status = input.status ? requireWorkItemStatus(input.status) : "ready";
    const priority = input.priority ? requireWorkItemPriority(input.priority) : "medium";
    const createdBy = requireWorkItemActorKind(input.createdBy);
    const category = clampCategory(input.category);
    const tags = clampTags(input.tags);
    const now = new Date().toISOString();
    const id = createId("wi");

    this.stateDb.transaction(() => {
      this.stateDb.run(
        `INSERT INTO work_items (
          id, thread_id, parent_id, kind, title, description, acceptance_criteria, status, priority,
          sort_index, created_by, created_at, updated_at, completed_at, category, tags
        ) VALUES (
          ?, NULL, NULL, ?, ?, ?, ?, ?, ?,
          (SELECT COALESCE(MAX(sort_index), -1) + 1 FROM work_items WHERE thread_id IS NULL AND parent_id IS NULL),
          ?, ?, ?, ?, ?, ?
        )`,
        id,
        kind,
        title,
        description,
        acceptance,
        status,
        priority,
        createdBy,
        now,
        now,
        status === "done" ? now : null,
        category,
        tags,
      );
      this.recordEvent({
        threadId: null,
        itemId: id,
        eventType: "created",
        actorKind: createdBy,
        actorId: input.actorId,
        payload: { title, kind, status, priority },
      });
    });
    const created = this.getBacklogItem(id);
    if (!created) throw new Error("failed to read back created backlog item");
    this.emitChange({ threadId: BACKLOG_SCOPE, kind: "created", itemId: id });
    return created;
  }

  updateBacklogItem(input: {
    itemId: string;
    title?: string;
    description?: string;
    acceptanceCriteria?: string | null;
    status?: WorkItemStatus;
    priority?: WorkItemPriority;
    category?: string | null;
    tags?: string | null;
    actorKind: WorkItemActorKind;
    actorId: string;
  }): void {
    const kind = requireWorkItemActorKind(input.actorKind);
    const now = new Date().toISOString();
    this.stateDb.transaction(() => {
      const existing = this.getBacklogItem(input.itemId);
      if (!existing) throw new Error(`backlog item ${input.itemId} not found`);
      const fields: string[] = [];
      const values: unknown[] = [];
      if (input.title !== undefined) { fields.push("title = ?"); values.push(requireTitle(input.title)); }
      if (input.description !== undefined) { fields.push("description = ?"); values.push(clampDescription(input.description)); }
      if (input.acceptanceCriteria !== undefined) { fields.push("acceptance_criteria = ?"); values.push(clampAcceptanceCriteria(input.acceptanceCriteria)); }
      if (input.status !== undefined) {
        const status = requireWorkItemStatus(input.status);
        fields.push("status = ?"); values.push(status);
        if (status === "done" || status === "canceled" || status === "archived") { fields.push("completed_at = ?"); values.push(now); }
        else { fields.push("completed_at = NULL"); }
      }
      if (input.priority !== undefined) { fields.push("priority = ?"); values.push(requireWorkItemPriority(input.priority)); }
      if (input.category !== undefined) { fields.push("category = ?"); values.push(clampCategory(input.category)); }
      if (input.tags !== undefined) { fields.push("tags = ?"); values.push(clampTags(input.tags)); }
      if (fields.length === 0) return;
      fields.push("updated_at = ?"); values.push(now);
      values.push(input.itemId);
      this.stateDb.run(
        `UPDATE work_items SET ${fields.join(", ")} WHERE thread_id IS NULL AND id = ?`,
        ...(values as Array<string | number | null>),
      );
      const after = this.getBacklogItem(input.itemId);
      this.recordEvent({
        threadId: null,
        itemId: input.itemId,
        eventType: "updated",
        actorKind: kind,
        actorId: input.actorId,
        payload: { before: existing, after },
      });
    });
    this.emitChange({ threadId: BACKLOG_SCOPE, kind: "updated", itemId: input.itemId });
  }

  deleteBacklogItem(itemId: string, actorKind: WorkItemActorKind, actorId: string): void {
    const kind = requireWorkItemActorKind(actorKind);
    const now = new Date().toISOString();
    let deleted = false;
    this.stateDb.transaction(() => {
      const existing = this.getBacklogItem(itemId);
      if (!existing) return;
      this.stateDb.run(
        `UPDATE work_items SET deleted_at = ?, updated_at = ? WHERE thread_id IS NULL AND id = ?`,
        now,
        now,
        itemId,
      );
      this.recordEvent({
        threadId: null,
        itemId,
        eventType: "deleted",
        actorKind: kind,
        actorId,
        payload: { before: existing },
      });
      deleted = true;
    });
    if (deleted) {
      this.emitChange({ threadId: BACKLOG_SCOPE, kind: "deleted", itemId });
    }
  }

  reorderBacklog(orderedItemIds: string[], actorKind: WorkItemActorKind, actorId: string): void {
    if (orderedItemIds.length === 0) return;
    if (new Set(orderedItemIds).size !== orderedItemIds.length) {
      throw new Error("reorder request contained duplicate ids");
    }
    const kind = requireWorkItemActorKind(actorKind);
    const now = new Date().toISOString();
    this.stateDb.transaction(() => {
      const rows = this.stateDb.all<{ id: string; parent_id: string | null }>(
        `SELECT id, parent_id FROM work_items
         WHERE thread_id IS NULL AND id IN (${orderedItemIds.map(() => "?").join(",")}) AND deleted_at IS NULL`,
        ...orderedItemIds,
      );
      if (rows.length !== orderedItemIds.length) {
        throw new Error("reorder request referenced unknown or deleted backlog items");
      }
      if (rows.some((row) => row.parent_id !== null)) {
        throw new Error("backlog reorder requires root-level items only");
      }
      for (let index = 0; index < orderedItemIds.length; index++) {
        this.stateDb.run(
          `UPDATE work_items SET sort_index = ?, updated_at = ? WHERE thread_id IS NULL AND id = ?`,
          index,
          now,
          orderedItemIds[index]!,
        );
      }
      this.recordEvent({
        threadId: null,
        itemId: null,
        eventType: "reordered",
        actorKind: kind,
        actorId,
        payload: { orderedItemIds },
      });
    });
    this.emitChange({ threadId: BACKLOG_SCOPE, kind: "reordered", itemId: null });
  }

  linkItems(threadId: string, fromItemId: string, toItemId: string, linkType: WorkItemLinkType): void {
    if (fromItemId === toItemId) throw new Error("cannot link a work item to itself");
    const kind = requireWorkItemLinkType(linkType);
    const now = new Date().toISOString();
    this.stateDb.transaction(() => {
      this.requireItemInThread(threadId, fromItemId, "from item");
      this.requireItemInThread(threadId, toItemId, "to item");
      this.stateDb.run(
        `INSERT INTO work_item_links (id, thread_id, from_item_id, to_item_id, link_type, created_at)
         VALUES (?, ?, ?, ?, ?, ?)`,
        createId("lnk"),
        threadId,
        fromItemId,
        toItemId,
        kind,
        now,
      );
      this.recordEvent({
        threadId,
        itemId: fromItemId,
        eventType: "linked",
        actorKind: "system",
        actorId: "work-item-store",
        payload: { fromItemId, toItemId, linkType: kind },
      });
    });
    this.emitChange({ threadId, kind: "linked", itemId: fromItemId });
  }

  listReady(threadId: string): WorkItem[] {
    const rows = this.stateDb.all<Record<string, unknown>>(
      `SELECT wi.*,
              (SELECT COUNT(*) FROM work_note WHERE work_note.work_item_id = wi.id) AS note_count
       FROM work_items wi
       WHERE wi.thread_id = ?
         AND wi.deleted_at IS NULL
         AND wi.status IN ('ready')
         AND NOT EXISTS (
           SELECT 1
           FROM work_item_links l
           JOIN work_items blocker ON blocker.id = l.from_item_id
           WHERE l.thread_id = wi.thread_id
             AND l.to_item_id = wi.id
             AND l.link_type = 'blocks'
             AND blocker.status NOT IN ('done', 'canceled', 'archived')
             AND blocker.deleted_at IS NULL
         )
       ORDER BY wi.priority DESC, wi.sort_index, wi.created_at`,
      threadId,
    );
    return rows.map(toWorkItem);
  }

  readWorkOptions(threadId: string, beforeSortIndex?: number): ReadWorkOptionsResult {
    const allReady = this.listReady(threadId);
    const ready = beforeSortIndex !== undefined
      ? allReady.filter((i) => i.sort_index < beforeSortIndex)
      : allReady;
    if (ready.length === 0) return { mode: "empty" };

    const head = ready[0]!;

    if (head.kind === "epic") {
      const childRows = this.stateDb.all<Record<string, unknown>>(
        `WITH RECURSIVE descendants(id) AS (
           SELECT id FROM work_items
           WHERE thread_id = ? AND parent_id = ? AND deleted_at IS NULL
           UNION ALL
           SELECT wi.id FROM work_items wi
           JOIN descendants d ON wi.parent_id = d.id
           WHERE wi.thread_id = ? AND wi.deleted_at IS NULL
         )
         SELECT wi.*,
                (SELECT COUNT(*) FROM work_note WHERE work_note.work_item_id = wi.id) AS note_count
         FROM work_items wi
         JOIN descendants d ON wi.id = d.id
         WHERE wi.status = 'ready'
           AND NOT EXISTS (
             SELECT 1
             FROM work_item_links l
             JOIN work_items blocker ON blocker.id = l.from_item_id
             WHERE l.thread_id = wi.thread_id
               AND l.to_item_id = wi.id
               AND l.link_type = 'blocks'
               AND blocker.status NOT IN ('done', 'canceled', 'archived')
               AND blocker.deleted_at IS NULL
           )
         ORDER BY wi.sort_index, wi.created_at`,
        threadId,
        head.id,
        threadId,
      );
      const children = childRows.map(toWorkItem);
      const withLinks = this.attachLinks(threadId, children);
      return { mode: "epic", epic: head, children: withLinks };
    }

    const standaloneItems = ready.filter((i) => i.kind !== "epic");
    const withLinks = this.attachLinks(threadId, standaloneItems);
    return { mode: "standalone", items: withLinks };
  }

  private attachLinks(threadId: string, items: WorkItem[]): WorkItemWithLinks[] {
    if (items.length === 0) return [];
    const ids = items.map((i) => i.id);
    const placeholders = ids.map(() => "?").join(",");
    const linkRows = this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM work_item_links
         WHERE thread_id = ? AND (from_item_id IN (${placeholders}) OR to_item_id IN (${placeholders}))
         ORDER BY created_at`,
        threadId,
        ...ids,
        ...ids,
      )
      .map(toWorkItemLink);
    return items.map((item) => ({
      item,
      outgoing: linkRows.filter((l) => l.from_item_id === item.id),
      incoming: linkRows.filter((l) => l.to_item_id === item.id),
    }));
  }

  listEvents(threadId: string, itemId?: string): WorkItemEvent[] {
    const rows = itemId
      ? this.stateDb.all<Record<string, unknown>>(
          `SELECT * FROM work_item_events WHERE thread_id = ? AND item_id = ? ORDER BY created_at DESC, id DESC`,
          threadId,
          itemId,
        )
      : this.stateDb.all<Record<string, unknown>>(
          `SELECT * FROM work_item_events WHERE thread_id = ? ORDER BY created_at DESC, id DESC`,
          threadId,
        );
    return rows.map(toWorkItemEvent);
  }

  getItemDetail(threadId: string, itemId: string, recentEventLimit = 20): WorkItemDetail | null {
    const item = this.getItem(threadId, itemId);
    if (!item) return null;
    const outgoing = this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM work_item_links WHERE thread_id = ? AND from_item_id = ? ORDER BY created_at`,
        threadId,
        itemId,
      )
      .map(toWorkItemLink);
    const incoming = this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM work_item_links WHERE thread_id = ? AND to_item_id = ? ORDER BY created_at`,
        threadId,
        itemId,
      )
      .map(toWorkItemLink);
    const recentEvents = this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM work_item_events WHERE thread_id = ? AND item_id = ?
         ORDER BY created_at DESC, id DESC LIMIT ?`,
        threadId,
        itemId,
        recentEventLimit,
      )
      .map(toWorkItemEvent);
    return { item, outgoing, incoming, recentEvents };
  }

  private requireItemInThread(threadId: string, itemId: string, label: string): void {
    const row = this.stateDb.get<{ id: string }>(
      `SELECT id FROM work_items WHERE thread_id = ? AND id = ? LIMIT 1`,
      threadId,
      itemId,
    );
    if (!row) throw new Error(`${label} ${itemId} not found in thread ${threadId}`);
  }

  private recordEvent(input: {
    threadId: string | null;
    itemId: string | null;
    eventType: string;
    actorKind: WorkItemActorKind;
    actorId: string;
    payload: unknown;
  }): void {
    this.stateDb.run(
      `INSERT INTO work_item_events (id, thread_id, item_id, event_type, actor_kind, actor_id, payload_json, created_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      createId("evt"),
      input.threadId,
      input.itemId,
      input.eventType,
      input.actorKind,
      input.actorId,
      JSON.stringify(input.payload ?? {}),
      new Date().toISOString(),
    );
  }
}

function toWorkItem(row: Record<string, unknown>): WorkItem {
  return {
    id: requireString(row, "id"),
    thread_id: row.thread_id == null ? null : String(row.thread_id),
    parent_id: row.parent_id == null ? null : String(row.parent_id),
    kind: requireWorkItemKind(requireString(row, "kind")),
    title: requireString(row, "title"),
    description: String(row.description ?? ""),
    status: requireWorkItemStatus(requireString(row, "status")),
    priority: requireWorkItemPriority(requireString(row, "priority")),
    sort_index: Number(row.sort_index ?? 0),
    created_by: requireWorkItemActorKind(requireString(row, "created_by")),
    created_at: requireString(row, "created_at"),
    updated_at: requireString(row, "updated_at"),
    completed_at: row.completed_at == null ? null : String(row.completed_at),
    deleted_at: row.deleted_at == null ? null : String(row.deleted_at),
    acceptance_criteria: row.acceptance_criteria == null ? null : String(row.acceptance_criteria),
    note_count: Number(row.note_count ?? 0),
    // Legacy pre-v29 rows used author='agent-auto'; the migration cancels
    // lingering in_progress ones but keeps the string on terminal rows for
    // audit. Map it to null so the narrowed WorkItemAuthor enum holds.
    author: row.author == null || String(row.author) === "agent-auto"
      ? null
      : requireOptionalWorkItemAuthor(String(row.author)),
    category: row.category == null ? null : String(row.category),
    tags: row.tags == null ? null : String(row.tags),
  };
}

function toWorkNote(row: Record<string, unknown>): WorkNote {
  return {
    id: requireString(row, "id"),
    work_item_id: row.work_item_id == null ? null : String(row.work_item_id),
    thread_id: row.thread_id == null ? null : String(row.thread_id),
    body: String(row.body ?? ""),
    author: String(row.author ?? ""),
    created_at: requireString(row, "created_at"),
  };
}

function toWorkItemLink(row: Record<string, unknown>): WorkItemLink {
  return {
    id: requireString(row, "id"),
    thread_id: requireString(row, "thread_id"),
    from_item_id: requireString(row, "from_item_id"),
    to_item_id: requireString(row, "to_item_id"),
    link_type: requireWorkItemLinkType(requireString(row, "link_type")),
    created_at: requireString(row, "created_at"),
  };
}

function toWorkItemEvent(row: Record<string, unknown>): WorkItemEvent {
  return {
    id: requireString(row, "id"),
    thread_id: row.thread_id == null ? null : String(row.thread_id),
    item_id: row.item_id == null ? null : String(row.item_id),
    event_type: requireString(row, "event_type"),
    actor_kind: requireWorkItemActorKind(requireString(row, "actor_kind")),
    actor_id: String(row.actor_id ?? ""),
    payload_json: String(row.payload_json ?? "{}"),
    created_at: requireString(row, "created_at"),
  };
}

function snapshotWorkItem(before: WorkItem, after: WorkItem) {
  return { before, after };
}

function requireString(row: Record<string, unknown>, key: string): string {
  const value = row[key];
  if (value == null || value === "") {
    throw new Error(`work item row is missing required field: ${key}`);
  }
  return String(value);
}

function requireTitle(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) throw new Error("work item title is required");
  if (trimmed.length > TITLE_MAX_LEN) {
    throw new Error(`work item title too long: max ${TITLE_MAX_LEN} chars`);
  }
  return trimmed;
}

function clampDescription(raw: string | undefined): string {
  const trimmed = raw?.trim() ?? "";
  if (trimmed.length > DESCRIPTION_MAX_LEN) {
    throw new Error(`work item description too long: max ${DESCRIPTION_MAX_LEN} chars`);
  }
  return trimmed;
}

function clampAcceptanceCriteria(raw: string | null | undefined): string | null {
  if (raw == null) return null;
  const trimmed = raw.trim();
  if (!trimmed) return null;
  if (trimmed.length > ACCEPTANCE_CRITERIA_MAX_LEN) {
    throw new Error(`work item acceptance criteria too long: max ${ACCEPTANCE_CRITERIA_MAX_LEN} chars`);
  }
  return trimmed;
}

const CATEGORY_MAX_LEN = 200;
const TAGS_MAX_LEN = 500;

function clampCategory(raw: string | null | undefined): string | null {
  if (raw == null) return null;
  const trimmed = raw.trim();
  if (!trimmed) return null;
  if (trimmed.length > CATEGORY_MAX_LEN) {
    throw new Error(`work item category too long: max ${CATEGORY_MAX_LEN} chars`);
  }
  return trimmed;
}

function clampTags(raw: string | null | undefined): string | null {
  if (raw == null) return null;
  // Normalize: split on commas, trim each, drop empties, dedupe (case
  // sensitive — the user's casing wins), rejoin with ", ".
  const parts = raw
    .split(",")
    .map((p) => p.trim())
    .filter((p) => p.length > 0);
  if (parts.length === 0) return null;
  const seen = new Set<string>();
  const uniq: string[] = [];
  for (const p of parts) {
    if (seen.has(p)) continue;
    seen.add(p);
    uniq.push(p);
  }
  const joined = uniq.join(", ");
  if (joined.length > TAGS_MAX_LEN) {
    throw new Error(`work item tags too long: max ${TAGS_MAX_LEN} chars`);
  }
  return joined;
}

function requireWorkItemKind(value: string): WorkItemKind {
  if (!WORK_ITEM_KINDS.has(value as WorkItemKind)) {
    throw new Error(`invalid work item kind: ${value}`);
  }
  return value as WorkItemKind;
}

function requireWorkItemStatus(value: string): WorkItemStatus {
  if (!WORK_ITEM_STATUSES.has(value as WorkItemStatus)) {
    throw new Error(`invalid work item status: ${value}`);
  }
  return value as WorkItemStatus;
}

function requireWorkItemPriority(value: string): WorkItemPriority {
  if (!WORK_ITEM_PRIORITIES.has(value as WorkItemPriority)) {
    throw new Error(`invalid work item priority: ${value}`);
  }
  return value as WorkItemPriority;
}

function requireWorkItemLinkType(value: string): WorkItemLinkType {
  if (!WORK_ITEM_LINK_TYPES.has(value as WorkItemLinkType)) {
    throw new Error(`invalid work item link type: ${value}`);
  }
  return value as WorkItemLinkType;
}

function requireOptionalWorkItemAuthor(value: string | null): WorkItemAuthor {
  if (!WORK_ITEM_AUTHORS.has(value as WorkItemAuthor)) {
    throw new Error(`invalid work item author: ${value}`);
  }
  return value as WorkItemAuthor;
}

function requireWorkItemActorKind(value: string): WorkItemActorKind {
  if (!WORK_ITEM_ACTOR_KINDS.has(value as WorkItemActorKind)) {
    throw new Error(`invalid work item actor kind: ${value}`);
  }
  return value as WorkItemActorKind;
}
