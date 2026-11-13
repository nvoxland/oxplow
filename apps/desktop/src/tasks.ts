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
import type { Bucket, FieldDecl, Reads, SqlQueryResult, StageDecl } from "./tauri-bridge/generated/bindings.js";

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

// ---- the task model's stores: their stages and their own fields ----

export type { FieldDecl };

/** What the screens draw for one store: its stages and its own fields (how
 *  its ids look in text is the shared record stores', `recordStores.ts`).
 *  It says nothing of what the store can do: the UI offers
 *  what the store's action points offer. */
export interface TaskTrackerProfile {
  provider: string;
  fields: FieldDecl[];
  /** Its stages, in its order. */
  stages: StageDecl[];
}

/** Each store's profile, by store id (a task's `provider`). */
export type TaskTrackerProfiles = Record<string, TaskTrackerProfile>;

/** The profiles `v_model_stage`, `v_model_field_decl` (a store's fields
 *  are under its id as `namespace`; oxplow's `assignee` and `rank` are not
 *  a store's) and `v_model_store` say: one per store, from that store's
 *  own rows. No store: none. */
export function taskTrackerProfilesOf(stages: SqlQueryResult, fields: SqlQueryResult, stores?: SqlQueryResult): TaskTrackerProfiles {
  const at = (r: SqlQueryResult, row: SqlCell[], name: string) => {
    const i = r.columns.indexOf(name);
    return i < 0 ? null : (row[i] ?? null);
  };
  const ids = new Set<string>();
  for (const row of stages.rows) ids.add(String(at(stages, row, "store")));
  if (stores) for (const row of stores.rows) ids.add(String(at(stores, row, "store")));
  const profiles: TaskTrackerProfiles = {};
  for (const provider of ids) {
    profiles[provider] = {
      provider,
      fields: fields.rows
        .filter((row) => String(at(fields, row, "namespace")) === provider)
        .map((row) => {
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
    };
  }
  return profiles;
}

/** The profile of a task's own store; null when no store by that id. */
export function profileOfTask(profiles: TaskTrackerProfiles, task: Pick<Task, "provider">): TaskTrackerProfile | null {
  return profiles[task.provider] ?? null;
}

function jsonOf<T>(raw: SqlCell | null, fallback: T): T {
  try {
    const parsed: unknown = JSON.parse(String(raw ?? ""));
    return parsed && typeof parsed === "object" ? (parsed as T) : fallback;
  } catch {
    return fallback;
  }
}

/** The task model's profiles, and what was read. */
export async function readTaskTrackerProfiles(): Promise<{ profiles: TaskTrackerProfiles; reads: Reads }> {
  const stages = await querySql(
    "SELECT store, id, title, bucket FROM v_model_stage WHERE model = 'oxplow/task_record' ORDER BY store, position",
    [],
    500,
  );
  const fields = await querySql(
    "SELECT namespace, name, title, kind, \"values\", read_only FROM v_model_field_decl WHERE model = 'oxplow/task_record' AND namespace <> 'oxplow' ORDER BY namespace, name",
    [],
    500,
  );
  const stores = await querySql("SELECT store FROM v_model_store WHERE model = 'oxplow/task_record'", [], 50);
  return {
    profiles: taskTrackerProfilesOf(stages, fields, stores),
    reads: {
      datasets: [...new Set([...stages.reads.datasets, ...fields.reads.datasets, ...stores.reads.datasets])],
      tables: [...new Set([...stages.reads.tables, ...fields.reads.tables, ...stores.reads.tables])],
      measures: [...new Set([...stages.reads.measures, ...fields.reads.measures, ...stores.reads.measures])],
    },
  };
}
