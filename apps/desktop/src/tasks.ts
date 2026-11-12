/**
 * Tasks read through the task interface (`.context/tasks.md`):
 * `v_oxplow_task` and its views, over the task model: oxtask and any other
 * store's items, or none (nothing). A store's own fields are in `native`,
 * described by the fields it declares (`v_model_field_decl`), its stages by
 * `v_model_stage`. Each
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

/** One item of the task model, as `v_oxplow_task` holds it. */
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
  (SELECT count(*) FROM v_oxplow_task_comment c WHERE c.uri = w.uri) AS comment_count`;

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
  const res = await querySql(`SELECT ${COLUMNS} FROM v_oxplow_task w WHERE w.uri = ?1`, [ref], 1);
  return { item: itemsFromResult(res)[0] ?? null, reads: res.reads };
}

/** Several items by ref, in one read (titles, states). */
export async function readTasksByRef(refs: string[]): Promise<{ items: Task[]; reads: Reads }> {
  if (refs.length === 0) return { items: [], reads: NO_READS };
  const res = await querySql(
    `SELECT ${COLUMNS} FROM v_oxplow_task w WHERE w.uri IN (${refs.map((_, i) => `?${i + 1}`).join(", ")})`,
    refs,
    refs.length,
  );
  return { items: itemsFromResult(res), reads: res.reads };
}

// ---- the task model's stores: what they can do and their own fields ----

/** A task store's feature flags (the Rust `TaskTrackerFeatures`). */
export type { FieldDecl, TaskTrackerFeatures };

/** No feature declared. */
export const NO_FEATURES: Required<TaskTrackerFeatures> = {
  hierarchy: false,
  comments: false,
  links: false,
  delete: false,
  idempotent_writes: false,
};

/** What oxtask (the task model's store) does: items nest, take comments and
 *  links, and are deleted. A project with no live task store does none. */
const STORE_FEATURES: Required<TaskTrackerFeatures> = {
  hierarchy: true,
  comments: true,
  links: true,
  delete: true,
  idempotent_writes: false,
};

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

/** The profile `v_model_stage` and `v_model_field_decl` say: the first
 *  store's stages, in its order, and the stores' own fields (oxplow's
 *  `assignee` and `rank` are not a store's). No store: nothing. */
export function taskTrackerProfileOf(stages: SqlQueryResult, fields: SqlQueryResult): TaskTrackerProfile {
  const at = (r: SqlQueryResult, row: SqlCell[], name: string) => {
    const i = r.columns.indexOf(name);
    return i < 0 ? null : (row[i] ?? null);
  };
  if (stages.rows.length === 0) return { provider: null, features: NO_FEATURES, fields: [], stages: [], idPattern: null };
  const provider = String(at(stages, stages.rows[0], "store"));
  return {
    provider,
    features: STORE_FEATURES,
    fields: fields.rows.map((row) => {
      const values = jsonOf<unknown>(at(fields, row, "values"), null);
      return {
        name: String(at(fields, row, "name")),
        title: String(at(fields, row, "title")),
        kind: String(at(fields, row, "kind")),
        values: Array.isArray(values) ? values.map(String) : [],
        read_only: Number(at(fields, row, "read_only") ?? 0) === 1,
      } as FieldDecl;
    }),
    stages: stages.rows
      .filter((row) => String(at(stages, row, "store")) === provider)
      .map((row) => ({
        id: String(at(stages, row, "id")),
        title: String(at(stages, row, "title")),
        bucket: String(at(stages, row, "bucket")) as Bucket,
      })),
    // The model does not say how a store's ids look (yet).
    idPattern: null,
  };
}

function jsonOf<T>(raw: SqlCell | null, fallback: T): T {
  try {
    const parsed: unknown = JSON.parse(String(raw ?? ""));
    return parsed && typeof parsed === "object" ? (parsed as T) : fallback;
  } catch {
    return fallback;
  }
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

/** The task model's profile, and what was read. */
export async function readTaskTrackerProfile(): Promise<{ profile: TaskTrackerProfile; reads: Reads }> {
  const stages = await querySql(
    "SELECT store, id, title, bucket FROM v_model_stage WHERE model = 'oxplow/task_record' ORDER BY store, position",
    [],
    200,
  );
  const fields = await querySql(
    "SELECT name, title, kind, \"values\", read_only FROM v_model_field_decl WHERE model = 'oxplow/task_record' AND namespace <> 'oxplow' ORDER BY name",
    [],
    200,
  );
  return {
    profile: taskTrackerProfileOf(stages, fields),
    reads: {
      datasets: [...new Set([...stages.reads.datasets, ...fields.reads.datasets])],
      tables: [...new Set([...stages.reads.tables, ...fields.reads.tables])],
      measures: [...new Set([...stages.reads.measures, ...fields.reads.measures])],
    },
  };
}
