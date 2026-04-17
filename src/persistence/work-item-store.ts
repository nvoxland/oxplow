import type { Logger } from "../core/logger.js";
import { createId } from "../core/ids.js";
import { getStateDatabase } from "./state-db.js";

export type WorkItemKind = "epic" | "task" | "subtask" | "bug" | "note";
export type WorkItemStatus = "waiting" | "ready" | "in_progress" | "blocked" | "done" | "canceled";
export type WorkItemPriority = "low" | "medium" | "high" | "urgent";
export type WorkItemLinkType = "blocks" | "relates_to" | "discovered_from";
export type WorkItemActorKind = "user" | "agent" | "system";

const WORK_ITEM_KINDS: ReadonlySet<WorkItemKind> = new Set([
  "epic", "task", "subtask", "bug", "note",
]);
const WORK_ITEM_STATUSES: ReadonlySet<WorkItemStatus> = new Set([
  "waiting", "ready", "in_progress", "blocked", "done", "canceled",
]);
const WORK_ITEM_PRIORITIES: ReadonlySet<WorkItemPriority> = new Set([
  "low", "medium", "high", "urgent",
]);
const WORK_ITEM_LINK_TYPES: ReadonlySet<WorkItemLinkType> = new Set([
  "blocks", "relates_to", "discovered_from",
]);
const WORK_ITEM_ACTOR_KINDS: ReadonlySet<WorkItemActorKind> = new Set([
  "user", "agent", "system",
]);

// External input caps — keep noisy payloads from corrupting the store or
// blowing up memory when reached from MCP/IPC. Not internal limits.
const TITLE_MAX_LEN = 500;
const DESCRIPTION_MAX_LEN = 20_000;
const NOTE_MAX_LEN = 20_000;

export interface WorkItem {
  id: string;
  batch_id: string;
  parent_id: string | null;
  kind: WorkItemKind;
  title: string;
  description: string;
  status: WorkItemStatus;
  priority: WorkItemPriority;
  sort_index: number;
  created_by: WorkItemActorKind;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
  deleted_at: string | null;
}

export interface WorkItemEvent {
  id: string;
  batch_id: string;
  item_id: string | null;
  event_type: string;
  actor_kind: WorkItemActorKind;
  actor_id: string;
  payload_json: string;
  created_at: string;
}

export type WorkItemChangeKind = "created" | "updated" | "note" | "linked" | "deleted" | "reordered";

export interface WorkItemChange {
  batchId: string;
  kind: WorkItemChangeKind;
  itemId: string | null;
}

export interface BatchWorkState {
  batchId: string;
  waiting: WorkItem[];
  inProgress: WorkItem[];
  done: WorkItem[];
  epics: WorkItem[];
  items: WorkItem[];
}

interface CreateWorkItemInput {
  batchId: string;
  parentId?: string | null;
  kind: WorkItemKind;
  title: string;
  description?: string;
  status?: WorkItemStatus;
  priority?: WorkItemPriority;
  createdBy: WorkItemActorKind;
  actorId: string;
}

interface UpdateWorkItemInput {
  batchId: string;
  itemId: string;
  title?: string;
  description?: string;
  status?: WorkItemStatus;
  priority?: WorkItemPriority;
  parentId?: string | null;
  actorKind: WorkItemActorKind;
  actorId: string;
}

export class WorkItemStore {
  private readonly stateDb;
  private readonly listeners = new Set<(change: WorkItemChange) => void>();

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
  }

  subscribe(listener: (change: WorkItemChange) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private emitChange(change: WorkItemChange): void {
    for (const listener of this.listeners) {
      try {
        listener(change);
      } catch (error) {
        this.logger?.warn("work item change listener threw", {
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }
  }

  getState(batchId: string): BatchWorkState {
    const items = this.listItems(batchId);
    return {
      batchId,
      waiting: items.filter((item) => item.status === "waiting" || item.status === "ready" || item.status === "blocked"),
      inProgress: items.filter((item) => item.status === "in_progress"),
      done: items.filter((item) => item.status === "done" || item.status === "canceled"),
      epics: items.filter((item) => item.kind === "epic"),
      items,
    };
  }

  listItems(batchId: string): WorkItem[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM work_items
         WHERE batch_id = ? AND deleted_at IS NULL
         ORDER BY sort_index, created_at, id`,
        batchId,
      )
      .map(toWorkItem);
  }

  getItem(batchId: string, itemId: string): WorkItem | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT * FROM work_items
       WHERE batch_id = ? AND id = ? AND deleted_at IS NULL
       LIMIT 1`,
      batchId,
      itemId,
    );
    return row ? toWorkItem(row) : null;
  }

  createItem(input: CreateWorkItemInput): WorkItem {
    const title = requireTitle(input.title);
    const description = clampDescription(input.description);
    const kind = requireWorkItemKind(input.kind);
    const status = input.status ? requireWorkItemStatus(input.status) : "waiting";
    const priority = input.priority ? requireWorkItemPriority(input.priority) : "medium";
    const createdBy = requireWorkItemActorKind(input.createdBy);
    const parentId = input.parentId ?? null;
    const now = new Date().toISOString();
    const id = createId("wi");

    const item: WorkItem = {
      id,
      batch_id: input.batchId,
      parent_id: parentId,
      kind,
      title,
      description,
      status,
      priority,
      sort_index: 0, // filled in by the INSERT subquery
      created_by: createdBy,
      created_at: now,
      updated_at: now,
      completed_at: status === "done" ? now : null,
      deleted_at: null,
    };

    this.stateDb.transaction(() => {
      if (parentId) this.requireItemInBatch(input.batchId, parentId, "parent");

      // Compute sort_index atomically inside the INSERT so concurrent creates
      // under the same parent can't collide on MAX+1.
      const parentClause = parentId ? "parent_id = ?" : "parent_id IS NULL";
      const sortParams: [string, ...(string[])] = parentId ? [input.batchId, parentId] : [input.batchId];
      this.stateDb.run(
        `INSERT INTO work_items (
          id, batch_id, parent_id, kind, title, description, status, priority,
          sort_index, created_by, created_at, updated_at, completed_at
        ) VALUES (
          ?, ?, ?, ?, ?, ?, ?, ?,
          (SELECT COALESCE(MAX(sort_index), -1) + 1 FROM work_items WHERE batch_id = ? AND ${parentClause}),
          ?, ?, ?, ?
        )`,
        item.id,
        item.batch_id,
        item.parent_id,
        item.kind,
        item.title,
        item.description,
        item.status,
        item.priority,
        ...sortParams,
        item.created_by,
        item.created_at,
        item.updated_at,
        item.completed_at,
      );
      const stored = this.getItem(input.batchId, item.id);
      if (!stored) throw new Error("work item was not persisted");
      item.sort_index = stored.sort_index;
      this.recordEvent({
        batchId: input.batchId,
        itemId: item.id,
        eventType: "created",
        actorKind: createdBy,
        actorId: input.actorId,
        payload: item,
      });
    });
    this.logger?.info("created work item", {
      batchId: item.batch_id,
      itemId: item.id,
      kind: item.kind,
      status: item.status,
    });
    this.emitChange({ batchId: item.batch_id, kind: "created", itemId: item.id });
    return item;
  }

  updateItem(input: UpdateWorkItemInput): WorkItem {
    const existing = this.getItem(input.batchId, input.itemId);
    if (!existing) throw new Error(`unknown work item: ${input.itemId}`);
    const actorKind = requireWorkItemActorKind(input.actorKind);
    const now = new Date().toISOString();
    const nextParentId = input.parentId !== undefined ? input.parentId : existing.parent_id;
    const nextStatus = input.status ? requireWorkItemStatus(input.status) : existing.status;
    const nextPriority = input.priority ? requireWorkItemPriority(input.priority) : existing.priority;
    const nextTitle = input.title !== undefined ? requireTitle(input.title) : existing.title;
    const nextDescription = input.description !== undefined ? clampDescription(input.description) : existing.description;

    const updated: WorkItem = {
      ...existing,
      parent_id: nextParentId,
      title: nextTitle,
      description: nextDescription,
      status: nextStatus,
      priority: nextPriority,
      updated_at: now,
      completed_at: nextStatus === "done"
        ? (existing.completed_at ?? now)
        : existing.status === "done"
          ? null
          : existing.completed_at,
    };
    if (nextParentId && nextParentId === input.itemId) {
      throw new Error("work item cannot be its own parent");
    }

    this.stateDb.transaction(() => {
      if (nextParentId && nextParentId !== existing.parent_id) {
        this.requireItemInBatch(input.batchId, nextParentId, "parent");
      }
      this.stateDb.run(
        `UPDATE work_items
         SET parent_id = ?, title = ?, description = ?, status = ?, priority = ?, updated_at = ?, completed_at = ?
         WHERE batch_id = ? AND id = ?`,
        updated.parent_id,
        updated.title,
        updated.description,
        updated.status,
        updated.priority,
        updated.updated_at,
        updated.completed_at,
        updated.batch_id,
        updated.id,
      );
      this.recordEvent({
        batchId: input.batchId,
        itemId: input.itemId,
        eventType: "updated",
        actorKind,
        actorId: input.actorId,
        payload: snapshotWorkItem(existing, updated),
      });
    });
    this.emitChange({ batchId: input.batchId, kind: "updated", itemId: input.itemId });
    return updated;
  }

  addNote(
    batchId: string,
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
      if (itemId) this.requireItemInBatch(batchId, itemId, "note target");
      this.recordEvent({
        batchId,
        itemId,
        eventType: "note",
        actorKind: kind,
        actorId,
        payload: { note: trimmed },
      });
    });
    this.emitChange({ batchId, kind: "note", itemId });
  }

  deleteItem(batchId: string, itemId: string, actorKind: WorkItemActorKind, actorId: string): void {
    const kind = requireWorkItemActorKind(actorKind);
    const now = new Date().toISOString();
    let deleted = false;
    this.stateDb.transaction(() => {
      const existing = this.getItem(batchId, itemId);
      if (!existing) return;
      this.stateDb.run(
        `UPDATE work_items SET deleted_at = ?, updated_at = ? WHERE batch_id = ? AND id = ?`,
        now,
        now,
        batchId,
        itemId,
      );
      this.recordEvent({
        batchId,
        itemId,
        eventType: "deleted",
        actorKind: kind,
        actorId,
        payload: { before: existing },
      });
      deleted = true;
    });
    if (deleted) {
      this.logger?.info("deleted work item", { batchId, itemId });
      this.emitChange({ batchId, kind: "deleted", itemId });
    }
  }

  reorderItems(
    batchId: string,
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
         WHERE batch_id = ? AND id IN (${orderedItemIds.map(() => "?").join(",")}) AND deleted_at IS NULL`,
        batchId,
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
          `UPDATE work_items SET sort_index = ?, updated_at = ? WHERE batch_id = ? AND id = ?`,
          index,
          now,
          batchId,
          orderedItemIds[index]!,
        );
      }
      this.recordEvent({
        batchId,
        itemId: null,
        eventType: "reordered",
        actorKind: kind,
        actorId,
        payload: { orderedItemIds },
      });
    });
    this.emitChange({ batchId, kind: "reordered", itemId: null });
  }

  linkItems(batchId: string, fromItemId: string, toItemId: string, linkType: WorkItemLinkType): void {
    if (fromItemId === toItemId) throw new Error("cannot link a work item to itself");
    const kind = requireWorkItemLinkType(linkType);
    const now = new Date().toISOString();
    this.stateDb.transaction(() => {
      this.requireItemInBatch(batchId, fromItemId, "from item");
      this.requireItemInBatch(batchId, toItemId, "to item");
      this.stateDb.run(
        `INSERT INTO work_item_links (id, batch_id, from_item_id, to_item_id, link_type, created_at)
         VALUES (?, ?, ?, ?, ?, ?)`,
        createId("lnk"),
        batchId,
        fromItemId,
        toItemId,
        kind,
        now,
      );
      this.recordEvent({
        batchId,
        itemId: fromItemId,
        eventType: "linked",
        actorKind: "system",
        actorId: "work-item-store",
        payload: { fromItemId, toItemId, linkType: kind },
      });
    });
    this.emitChange({ batchId, kind: "linked", itemId: fromItemId });
  }

  listReady(batchId: string): WorkItem[] {
    const rows = this.stateDb.all<Record<string, unknown>>(
      `SELECT wi.*
       FROM work_items wi
       WHERE wi.batch_id = ?
         AND wi.deleted_at IS NULL
         AND wi.status IN ('waiting', 'ready')
         AND NOT EXISTS (
           SELECT 1
           FROM work_item_links l
           JOIN work_items blocker ON blocker.id = l.from_item_id
           WHERE l.batch_id = wi.batch_id
             AND l.to_item_id = wi.id
             AND l.link_type = 'blocks'
             AND blocker.status NOT IN ('done', 'canceled')
             AND blocker.deleted_at IS NULL
         )
       ORDER BY wi.priority DESC, wi.sort_index, wi.created_at`,
      batchId,
    );
    return rows.map(toWorkItem);
  }

  listEvents(batchId: string, itemId?: string): WorkItemEvent[] {
    const rows = itemId
      ? this.stateDb.all<Record<string, unknown>>(
          `SELECT * FROM work_item_events WHERE batch_id = ? AND item_id = ? ORDER BY created_at DESC, id DESC`,
          batchId,
          itemId,
        )
      : this.stateDb.all<Record<string, unknown>>(
          `SELECT * FROM work_item_events WHERE batch_id = ? ORDER BY created_at DESC, id DESC`,
          batchId,
        );
    return rows.map(toWorkItemEvent);
  }

  private requireItemInBatch(batchId: string, itemId: string, label: string): void {
    const row = this.stateDb.get<{ id: string }>(
      `SELECT id FROM work_items WHERE batch_id = ? AND id = ? LIMIT 1`,
      batchId,
      itemId,
    );
    if (!row) throw new Error(`${label} ${itemId} not found in batch ${batchId}`);
  }

  private recordEvent(input: {
    batchId: string;
    itemId: string | null;
    eventType: string;
    actorKind: WorkItemActorKind;
    actorId: string;
    payload: unknown;
  }): void {
    this.stateDb.run(
      `INSERT INTO work_item_events (id, batch_id, item_id, event_type, actor_kind, actor_id, payload_json, created_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      createId("evt"),
      input.batchId,
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
    batch_id: requireString(row, "batch_id"),
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
  };
}

function toWorkItemEvent(row: Record<string, unknown>): WorkItemEvent {
  return {
    id: requireString(row, "id"),
    batch_id: requireString(row, "batch_id"),
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

function requireWorkItemActorKind(value: string): WorkItemActorKind {
  if (!WORK_ITEM_ACTOR_KINDS.has(value as WorkItemActorKind)) {
    throw new Error(`invalid work item actor kind: ${value}`);
  }
  return value as WorkItemActorKind;
}
