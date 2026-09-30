/// Reading a `query_sql` result by column name — the one helper every
/// SQL-backed reader (metrics, the explorer's catalog) maps rows with.
import type { SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** A result's rows as objects keyed by column name. */
export function rowObjects(result: SqlQueryResult): Record<string, SqlCell>[] {
  return result.rows.map((row) => Object.fromEntries(result.columns.map((c, i) => [c, row[i]])));
}

/** A cell as text; `null` for SQL NULL. */
export const cellText = (v: SqlCell | undefined): string | null => (v === null || v === undefined ? null : String(v));

/** A cell as a number; `null` when it isn't one. */
export const cellNumber = (v: SqlCell | undefined): number | null => (typeof v === "number" ? v : null);
