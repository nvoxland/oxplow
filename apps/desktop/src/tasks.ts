/**
 * Tasks read through the task interface (`.context/tasks.md`):
 * `v_task` and its views, whichever list is active — oxtask,
 * an extension's tracker, or none (nothing). The active list's own fields
 * are in `native`, described by the fields it declares
 * (`v_capability_provider.fields`); what it can do, by its features. Each
 * read returns what it read (`reads`) so a page re-runs with
 * `useRerunOnChange`.
 */
import { querySql, type SqlCell } from "./api.js";
import { NO_READS } from "./viz/rerun.js";
import { taskUri } from "./uri/uri.js";
import type { Bucket, FieldDecl, Reads, SqlQueryResult, StageDecl, TaskTrackerFeatures } from "./tauri-bridge/generated/bindings.js";

/** One of core's six buckets, each of a list's stages in one: what oxplow
 *  acts on. The UI draws the list's stages. */
export type { Bucket, StageDecl };

/** One item on the active task tracker, as `v_task` holds it. */
export interface Task {
  /** `task:<provider>:<id>`: its identity everywhere. */
  uri: string;
  provider: string;
  title: string;
  body: string;
  /** Its stage's bucket. */
  state: Bucket;
  /** The stage it is in, one its list declares. */
  stage: string;
  /** How the list names that stage (its id when it doesn't). */
  stageTitle: string;
  /** Its parent's ref (an epic), when the list nests. */
  parentUri: string | null;
  /** Who is going to work on it: a thread's or the person's URI; null
   *  for nobody (and for a done or canceled item). Core's record. */
  assignee: string | null;
  /** Its place in its assignee's queue (ascending); null orders by
   *  creation. */
  rank: number | null;
  /** When it reached done or canceled. */
  closedAt: string | null;
  createdAt: string;
  updatedAt: string;
  /** The list's own fields, by the names it declares. */
  native: Record<string, unknown>;
  commentCount: number;
}

const COLUMNS = `w.uri, w.provider, w.title, w.body, w.state, w.stage, w.stage_title, w.parent_uri, w.assignee, w.rank, w.closed_at,
  w.created_at, w.updated_at, w.native,
  (SELECT count(*) FROM v_task_comment c WHERE c.uri = w.uri) AS comment_count`;

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

export function itemsFromResult(result: SqlQueryResult): Task[] {
  const col = (name: string) => result.columns.indexOf(name);
  const at = (row: SqlCell[], name: string) => row[col(name)];
  return result.rows.map((row) => {
    const rank = at(row, "rank");
    return {
      uri: String(at(row, "uri")),
      provider: String(at(row, "provider")),
      title: String(at(row, "title") ?? ""),
      body: String(at(row, "body") ?? ""),
      state: String(at(row, "state")) as Bucket,
      stage: String(at(row, "stage") ?? ""),
      stageTitle: String(at(row, "stage_title") ?? at(row, "stage") ?? ""),
      parentUri: text(at(row, "parent_uri")),
      assignee: text(at(row, "assignee")),
      rank: rank === null || rank === undefined ? null : Number(rank),
      closedAt: text(at(row, "closed_at")),
      createdAt: String(at(row, "created_at") ?? ""),
      updatedAt: String(at(row, "updated_at") ?? ""),
      native: nativeOf(at(row, "native")),
      commentCount: Number(at(row, "comment_count") ?? 0),
    };
  });
}

/** One task by ref, with what it read. */
export async function readTask(ref: string): Promise<{ item: Task | null; reads: Reads }> {
  const res = await querySql(`SELECT ${COLUMNS} FROM v_task w WHERE w.uri = ?1`, [ref], 1);
  return { item: itemsFromResult(res)[0] ?? null, reads: res.reads };
}

/** Several items by ref, in one read (titles, states). */
export async function readTasksByRef(refs: string[]): Promise<{ items: Task[]; reads: Reads }> {
  if (refs.length === 0) return { items: [], reads: NO_READS };
  const res = await querySql(
    `SELECT ${COLUMNS} FROM v_task w WHERE w.uri IN (${refs.map((_, i) => `?${i + 1}`).join(", ")})`,
    refs,
    refs.length,
  );
  return { items: itemsFromResult(res), reads: res.reads };
}

// ---- the active list: what it can do and its own fields (v_capability_provider) ----

/** A task tracker provider's feature flags, as the provider declares them
 *  (the Rust `TaskTrackerFeatures`). */
export type { FieldDecl, TaskTrackerFeatures };

/** No feature declared. */
export const NO_FEATURES: Required<TaskTrackerFeatures> = {
  hierarchy: false,
  comments: false,
  links: false,
  delete: false,
  idempotent_writes: false,
};

/** One capability's provider, with its flags and fields as declared. */
export interface CapabilityProvider {
  capability: string;
  provider: string;
  /** The extension it comes from; null for oxplow's own. */
  extension: string | null;
  features: Record<string, unknown>;
  /** A task tracker's own fields. */
  fields: FieldDecl[];
  /** A task tracker's stages, in its order. */
  stages: StageDecl[];
  /** What a task tracker's own ids look like (a regex matched whole). */
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
    const stages = jsonOf<StageDecl[]>(at(row, "stages"), []);
    return {
      capability: String(at(row, "capability")),
      provider: String(at(row, "provider")),
      extension: at(row, "extension") == null ? null : String(at(row, "extension")),
      features: jsonOf<Record<string, unknown>>(at(row, "features"), {}),
      fields: Array.isArray(fields) ? fields : [],
      stages: Array.isArray(stages) ? stages : [],
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
    "SELECT capability, provider, extension, features, fields, stages, id_pattern, active FROM v_capability_provider WHERE capability = ?1",
    [capability],
    100,
  );
  return { providers: capabilityProvidersFromResult(res), reads: res.reads };
}

/** The capability's active task tracker provider — where every create files
 *  — or null while none is listed. */
export function activeProviderOf(providers: CapabilityProvider[]): string | null {
  return providers.find((p) => p.capability === "task_tracker" && p.active)?.provider ?? null;
}

/** A task tracker provider's flags; every flag off for one that isn't
 *  listed or doesn't declare it, so the UI only offers what it can do. */
export function featuresFor(providers: CapabilityProvider[], provider: string): Required<TaskTrackerFeatures> {
  const f = providers.find((p) => p.capability === "task_tracker" && p.provider === provider)?.features ?? {};
  return {
    hierarchy: f.hierarchy === true,
    comments: f.comments === true,
    links: f.links === true,
    delete: f.delete === true,
    idempotent_writes: f.idempotent_writes === true,
  };
}

/** What the screens offer: the active list, what it can do and its own
 *  fields. */
export interface TaskTrackerProfile {
  provider: string | null;
  features: Required<TaskTrackerFeatures>;
  fields: FieldDecl[];
  /** Its stages, in its order. */
  stages: StageDecl[];
  /** How its own ids look in text; null when it doesn't say (none). */
  idPattern: string | null;
}

export function taskTrackerProfileOf(providers: CapabilityProvider[]): TaskTrackerProfile {
  const provider = activeProviderOf(providers);
  if (provider === null) return { provider: null, features: NO_FEATURES, fields: [], stages: [], idPattern: null };
  const row = providers.find((p) => p.capability === "task_tracker" && p.provider === provider);
  return {
    provider,
    features: featuresFor(providers, provider),
    fields: row?.fields ?? [],
    stages: row?.stages ?? [],
    idPattern: row?.idPattern ?? null,
  };
}

/** The task ref a loose id in text names on the active list (`tsk42`
 *  with oxtask → `task:oxplow:tsk42`); null when it isn't
 *  one of that list's ids, or the list says nothing about its ids. */
export function taskRefOfMention(profile: TaskTrackerProfile, id: string): string | null {
  if (!profile.provider || !profile.idPattern) return null;
  let re: RegExp;
  try {
    re = new RegExp(`^(?:${profile.idPattern})$`);
  } catch {
    return null;
  }
  return re.test(id) ? taskUri(profile.provider, id) : null;
}

/** The active list's profile, and what was read. */
export async function readTaskTrackerProfile(): Promise<{ profile: TaskTrackerProfile; reads: Reads }> {
  const { providers, reads } = await readCapabilityProviders("task_tracker");
  return { profile: taskTrackerProfileOf(providers), reads };
}
