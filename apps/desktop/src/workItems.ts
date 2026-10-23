/**
 * Work items read through the work-item interface (`.context/work-items.md`):
 * `v_work_item` and its views, whichever list is active — oxplow's tasks,
 * an extension's tracker, or none (nothing). The active list's own fields
 * are in `native`, described by the fields it declares
 * (`v_capability_provider.fields`); what it can do, by its features. Each
 * read returns what it read (`reads`) so a page re-runs with
 * `useRerunOnChange`.
 */
import { querySql, type SqlCell } from "./api.js";
import { NO_READS } from "./lens/lensRerun.js";
import { threadIdOf, threadRowId } from "./modelIds.js";
import { workItemUri } from "./uri/uri.js";
import type { FieldDecl, Reads, SqlQueryResult, WorkItemsFeatures } from "./tauri-bridge/generated/bindings.js";

/** The state every provider maps to. */
export type CanonicalState = "todo" | "in_progress" | "blocked" | "done" | "canceled";


/** One item on the active work list, as `v_work_item` holds it. */
export interface WorkItem {
  /** `work_item:<provider>:<id>`: its identity everywhere. */
  uri: string;
  provider: string;
  title: string;
  body: string;
  state: CanonicalState;
  /** Its parent's ref (an epic), when the list nests. */
  parentUri: string | null;
  /** The thread whose list it's on (`thr3`); null on the backlog. */
  threadId: string | null;
  /** Its place on its list (ascending); null orders by creation. */
  rank: number | null;
  /** When it reached done or canceled. */
  closedAt: string | null;
  createdAt: string;
  updatedAt: string;
  /** The list's own fields, by the names it declares. */
  native: Record<string, unknown>;
  commentCount: number;
}

/** A list — a thread's, or the backlog's (`threadId` null) — in list
 *  order, and by state: an item with a child on the list is an epic;
 *  `done` holds done and canceled. */
export interface WorkList {
  threadId: string | null;
  /** Every item, in list order. */
  all: WorkItem[];
  epics: WorkItem[];
  /** Ready to start (`todo`). */
  items: WorkItem[];
  inProgress: WorkItem[];
  /** Blocked. */
  waiting: WorkItem[];
  done: WorkItem[];
  /** What the read read: a change to one of these models re-reads it. */
  reads: Reads;
}

const COLUMNS = `w.uri, w.provider, w.title, w.body, w.state, w.parent_uri, w.thread_id, w.rank, w.closed_at,
  w.created_at, w.updated_at, w.native,
  (SELECT count(*) FROM v_work_item_comment c WHERE c.uri = w.uri) AS comment_count`;

const ORDER = "ORDER BY w.rank IS NULL, w.rank, w.created_at";

export type WorkItemScope = { thread: string } | "backlog" | "all";

/** The SQL for a list: a thread's, the backlog's, or every item; in list
 *  order (rank, then oldest first). */
export function workItemsQuery(opts: { scope: WorkItemScope; states?: CanonicalState[] }): { sql: string; params: SqlCell[] } {
  const where: string[] = [];
  const params: SqlCell[] = [];
  if (opts.scope === "backlog") where.push("w.thread_id IS NULL");
  else if (opts.scope !== "all") {
    params.push(threadRowId(opts.scope.thread));
    where.push("w.thread_id = ?1");
  }
  if (opts.states && opts.states.length > 0) {
    where.push(`w.state IN (${opts.states.map((s) => `'${s}'`).join(", ")})`);
  }
  const sql = `SELECT ${COLUMNS} FROM v_work_item w${where.length ? ` WHERE ${where.join(" AND ")}` : ""}
    ${ORDER}`;
  return { sql, params };
}

const text = (c: SqlCell | undefined): string | null => (c === null || c === undefined ? null : String(c));

function nativeOf(raw: SqlCell | undefined): Record<string, unknown> {
  if (raw === null || raw === undefined) return {};
  try {
    const parsed: unknown = JSON.parse(String(raw));
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? (parsed as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

export function itemsFromResult(result: SqlQueryResult): WorkItem[] {
  const col = (name: string) => result.columns.indexOf(name);
  const at = (row: SqlCell[], name: string) => row[col(name)];
  return result.rows.map((row) => {
    const thread = at(row, "thread_id");
    const rank = at(row, "rank");
    return {
      uri: String(at(row, "uri")),
      provider: String(at(row, "provider")),
      title: String(at(row, "title") ?? ""),
      body: String(at(row, "body") ?? ""),
      state: String(at(row, "state")) as CanonicalState,
      parentUri: text(at(row, "parent_uri")),
      threadId: thread === null || thread === undefined ? null : threadIdOf(Number(thread)),
      rank: rank === null || rank === undefined ? null : Number(rank),
      closedAt: text(at(row, "closed_at")),
      createdAt: String(at(row, "created_at") ?? ""),
      updatedAt: String(at(row, "updated_at") ?? ""),
      native: nativeOf(at(row, "native")),
      commentCount: Number(at(row, "comment_count") ?? 0),
    };
  });
}

/** One work item by ref, with what it read. */
export async function readWorkItem(ref: string): Promise<{ item: WorkItem | null; reads: Reads }> {
  const res = await querySql(`SELECT ${COLUMNS} FROM v_work_item w WHERE w.uri = ?1`, [ref], 1);
  return { item: itemsFromResult(res)[0] ?? null, reads: res.reads };
}

/** Several items by ref, in one read (titles, states). */
export async function readWorkItemsByRef(refs: string[]): Promise<{ items: WorkItem[]; reads: Reads }> {
  if (refs.length === 0) return { items: [], reads: NO_READS };
  const res = await querySql(
    `SELECT ${COLUMNS} FROM v_work_item w WHERE w.uri IN (${refs.map((_, i) => `?${i + 1}`).join(", ")})`,
    refs,
    refs.length,
  );
  return { items: itemsFromResult(res), reads: res.reads };
}

/** A list of work items, with what it read. */
export async function readWorkItems(opts: {
  scope: WorkItemScope;
  states?: CanonicalState[];
}): Promise<{ items: WorkItem[]; reads: Reads }> {
  const q = workItemsQuery(opts);
  const res = await querySql(q.sql, q.params, 10_000);
  return { items: itemsFromResult(res), reads: res.reads };
}

/** Bucket a list's items (in list order) by state; an item with a child
 *  on the list is an epic. */
export function bucketWorkList(threadId: string | null, all: WorkItem[], reads: Reads): WorkList {
  const parents = new Set(all.map((i) => i.parentUri).filter((p): p is string => p !== null));
  const list: WorkList = { threadId, all, epics: [], items: [], inProgress: [], waiting: [], done: [], reads };
  for (const i of all) {
    if (parents.has(i.uri)) list.epics.push(i);
    else if (i.state === "blocked") list.waiting.push(i);
    else if (i.state === "in_progress") list.inProgress.push(i);
    else if (i.state === "todo") list.items.push(i);
    else list.done.push(i);
  }
  return list;
}

/** A thread's list (or the backlog's, `null`). */
export async function readWorkList(threadId: string | null): Promise<WorkList> {
  const { items, reads } = await readWorkItems({ scope: threadId ? { thread: threadId } : "backlog" });
  return bucketWorkList(threadId, items, reads);
}

// ---- the active list: what it can do and its own fields (v_capability_provider) ----

/** A work-items provider's feature flags, as the provider declares them
 *  (the Rust `WorkItemsFeatures`). */
export type { FieldDecl, WorkItemsFeatures };

/** No feature declared. */
export const NO_FEATURES: Required<WorkItemsFeatures> = {
  hierarchy: false,
  comments: false,
  links: false,
  delete: false,
  idempotent_writes: false,
  ordering: false,
  lists: false,
};

/** One capability's provider, with its flags and fields as declared. */
export interface CapabilityProvider {
  capability: string;
  provider: string;
  /** The extension it comes from; null for oxplow's own. */
  extension: string | null;
  features: Record<string, unknown>;
  /** A work list's own fields. */
  fields: FieldDecl[];
  /** What a work list's own ids look like (a regex matched whole). */
  idPattern: string | null;
  /** The capability's active provider. */
  active: boolean;
}

function jsonOf<T>(raw: SqlCell | null, fallback: T): T {
  try {
    const parsed: unknown = JSON.parse(String(raw ?? ""));
    return parsed && typeof parsed === "object" ? (parsed as T) : fallback;
  } catch {
    return fallback;
  }
}

export function capabilityProvidersFromResult(result: SqlQueryResult): CapabilityProvider[] {
  const at = (row: SqlCell[], name: string) => {
    const i = result.columns.indexOf(name);
    return i < 0 ? null : (row[i] ?? null);
  };
  return result.rows.map((row) => {
    const fields = jsonOf<FieldDecl[]>(at(row, "fields"), []);
    return {
      capability: String(at(row, "capability")),
      provider: String(at(row, "provider")),
      extension: at(row, "extension") == null ? null : String(at(row, "extension")),
      features: jsonOf<Record<string, unknown>>(at(row, "features"), {}),
      fields: Array.isArray(fields) ? fields : [],
      idPattern: at(row, "id_pattern") == null ? null : String(at(row, "id_pattern")),
      active: Number(at(row, "active") ?? 0) === 1,
    };
  });
}

/** `capability`'s providers, and what was read. */
export async function readCapabilityProviders(
  capability: string,
): Promise<{ providers: CapabilityProvider[]; reads: Reads }> {
  const res = await querySql(
    "SELECT capability, provider, extension, features, fields, id_pattern, active FROM v_capability_provider WHERE capability = ?1",
    [capability],
    100,
  );
  return { providers: capabilityProvidersFromResult(res), reads: res.reads };
}

/** The capability's active work-items provider — where every create files
 *  — or null while none is listed. */
export function activeProviderOf(providers: CapabilityProvider[]): string | null {
  return providers.find((p) => p.capability === "work_items" && p.active)?.provider ?? null;
}

/** A work-items provider's flags; every flag off for one that isn't
 *  listed or doesn't declare it, so the UI only offers what it can do. */
export function featuresFor(providers: CapabilityProvider[], provider: string): Required<WorkItemsFeatures> {
  const f = providers.find((p) => p.capability === "work_items" && p.provider === provider)?.features ?? {};
  return {
    hierarchy: f.hierarchy === true,
    comments: f.comments === true,
    links: f.links === true,
    delete: f.delete === true,
    idempotent_writes: f.idempotent_writes === true,
    ordering: f.ordering === true,
    lists: f.lists === true,
  };
}

/** What the screens offer: the active list, what it can do and its own
 *  fields. */
export interface WorkListProfile {
  provider: string | null;
  features: Required<WorkItemsFeatures>;
  fields: FieldDecl[];
  /** How its own ids look in text; null when it doesn't say (none). */
  idPattern: string | null;
}

export function workListProfileOf(providers: CapabilityProvider[]): WorkListProfile {
  const provider = activeProviderOf(providers);
  if (provider === null) return { provider: null, features: NO_FEATURES, fields: [], idPattern: null };
  const row = providers.find((p) => p.capability === "work_items" && p.provider === provider);
  return {
    provider,
    features: featuresFor(providers, provider),
    fields: row?.fields ?? [],
    idPattern: row?.idPattern ?? null,
  };
}

/** The work-item ref a loose id in text names on the active list (`tsk42`
 *  with oxplow's tasks → `work_item:oxplow:tsk42`); null when it isn't
 *  one of that list's ids, or the list says nothing about its ids. */
export function workItemRefOfMention(profile: WorkListProfile, id: string): string | null {
  if (!profile.provider || !profile.idPattern) return null;
  let re: RegExp;
  try {
    re = new RegExp(`^(?:${profile.idPattern})$`);
  } catch {
    return null;
  }
  return re.test(id) ? workItemUri(profile.provider, id) : null;
}

/** The active list's profile, and what was read. */
export async function readWorkListProfile(): Promise<{ profile: WorkListProfile; reads: Reads }> {
  const { providers, reads } = await readCapabilityProviders("work_items");
  return { profile: workListProfileOf(providers), reads };
}
