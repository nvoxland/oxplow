/// Lens params a person picks rather than types (tsk1043): oxplow's own id
/// params — the names it already fills from context (`stream_id`,
/// `thread_id`) and the ones a slot binds (`ref` for a work item, `effort_id`) — choose
/// from what exists, by title; any other param is typed.
import type { SqlCell, SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

export type ParamKind = "work_item" | "effort" | "thread" | "stream";

const KINDS: Record<string, ParamKind> = {
  ref: "work_item",
  effort_id: "effort",
  thread_id: "thread",
  stream_id: "stream",
};

/** The picker a param named `name` gets, if any. */
export function paramKind(name: string): ParamKind | null {
  return KINDS[name] ?? null;
}

/** What a picker lists: `value` (the id the query binds) and `label`,
 *  newest first. */
export function paramOptionsSql(kind: ParamKind): string {
  switch (kind) {
    case "work_item":
      return "SELECT ref AS value, title AS label FROM v_work_item ORDER BY updated_at DESC LIMIT 200";
    case "effort":
      return `SELECT e.id AS value,
                coalesce(w.title, e.title, 'Unlinked work') || ' · ' || substr(e.started_at, 1, 16) AS label
              FROM v_effort e LEFT JOIN v_work_item w ON w.ref = e.work_item
              ORDER BY e.started_at DESC LIMIT 200`;
    case "thread":
      return "SELECT id AS value, title AS label FROM v_thread WHERE closed_at IS NULL ORDER BY sort_index";
    case "stream":
      return "SELECT id AS value, title AS label FROM v_stream WHERE archived_at IS NULL ORDER BY id";
  }
}

export interface ParamOption {
  value: SqlCell;
  label: string;
}

export function paramOptions(result: SqlQueryResult): ParamOption[] {
  const v = result.columns.indexOf("value");
  const l = result.columns.indexOf("label");
  return result.rows.map((row) => {
    const value = row[v] ?? null;
    const label = row[l];
    return { value, label: label === null || label === undefined ? String(value) : String(label) };
  });
}
