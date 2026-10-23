/// Every setting's value now and where it comes from: `v_config_value`
/// (a live model — the project keys, a person's own layer, the AI roles,
/// what extensions declare). What Settings shows, and what the rest of the
/// window asks a setting's value of.
import { querySql } from "./api.js";
import type { Reads, SqlCell } from "./tauri-bridge/generated/bindings.js";

export type SettingOrigin = "default" | "global" | "project" | "personal" | "extension";

export interface EffectiveSetting {
  /** A project key (`snapshotRetentionDays`), or a scoped name
   *  (`ai.roles.main`, `personal.activeProviders`, `metrics.<key>`). */
  key: string;
  value: unknown;
  origin: SettingOrigin;
  /** The extension a value comes from (`origin: extension`). */
  extension: string | null;
  doc: string;
  /** A project key's value schema; null for the rest. */
  schema: unknown;
}

const COLUMNS = "key, value, origin, extension, doc, schema";

function parsed(cell: SqlCell): unknown {
  return typeof cell === "string" ? JSON.parse(cell) : null;
}

function settingOf(row: SqlCell[]): EffectiveSetting {
  const [key, value, origin, extension, doc, schema] = row;
  return {
    key: String(key),
    value: parsed(value),
    origin: origin as SettingOrigin,
    extension: typeof extension === "string" ? extension : null,
    doc: typeof doc === "string" ? doc : "",
    schema: parsed(schema),
  };
}

/** Every setting, with what the read read (to re-run on its change). */
export async function readSettings(): Promise<{ settings: EffectiveSetting[]; reads: Reads }> {
  const res = await querySql(`SELECT ${COLUMNS} FROM v_config_value ORDER BY key`, [], 10_000);
  return { settings: res.rows.map(settingOf), reads: res.reads };
}

/** One setting's value now (null when unset or unknown). */
export async function settingValue(key: string): Promise<unknown> {
  const res = await querySql(`SELECT ${COLUMNS} FROM v_config_value WHERE key = ?1`, [key], 1);
  const row = res.rows[0];
  return row ? settingOf(row).value : null;
}
