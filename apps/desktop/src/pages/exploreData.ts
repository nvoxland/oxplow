/// Explore Data's catalog, read through SQL (P4.10, tsk517): the model
/// registry is the catalog — `v_model` for what exists, `v_model_column`
/// for each model's documented columns. The pure half; the page runs the
/// queries.
import { cellText, rowObjects } from "../sqlRows.js";
import type { SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

/** A published model: core, an extension's SQL model, or a synced entity. */
export interface ModelRow {
  view: string;
  owner: string;
  /** `sql` or `entity`. */
  kind: string;
  description: string;
}

/** One documented column of a model. */
export interface ModelColumn {
  name: string;
  /** What SQLite reports; empty for a computed column. */
  sqlType: string;
  doc: string;
}

export const MODELS_SQL =
  "SELECT view, owner, kind, description FROM v_model ORDER BY owner <> 'core', owner, view";

export const MODEL_COLUMNS_SQL = "SELECT name, sql_type, doc FROM v_model_column WHERE view = ?1 ORDER BY position";

export function models(result: SqlQueryResult): ModelRow[] {
  return rowObjects(result).map((r) => ({
    view: cellText(r.view) ?? "",
    owner: cellText(r.owner) ?? "",
    kind: cellText(r.kind) ?? "",
    description: cellText(r.description) ?? "",
  }));
}

export function modelColumns(result: SqlQueryResult): ModelColumn[] {
  return rowObjects(result).map((r) => ({
    name: cellText(r.name) ?? "",
    sqlType: cellText(r.sql_type) ?? "",
    doc: cellText(r.doc) ?? "",
  }));
}
