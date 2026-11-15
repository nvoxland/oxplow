/// Every setting's value now and where it comes from: `v_config_value`
/// (the shared core and extension registry). What Settings shows, and what the rest of the
/// window asks a setting's value of.
import { querySql } from "./api.js";
import type { Reads, SqlCell } from "./tauri-bridge/generated/bindings.js";

export type SettingOrigin = "default" | "global" | "project" | "local";

export interface EffectiveSetting {
  /** A registered, fully qualified key such as `oxplow.snapshots.retentionDays`. */
  key: string;
  value: unknown;
  origin: SettingOrigin | null;
  /** The owning extension; null for core. */
  extension: string | null;
  doc: string;
  /** The registered value schema. */
  schema: unknown;
  title: string;
  category: string;
  secret: boolean;
  isSet: boolean;
  owner: string;
  restartRequired: boolean;
  ui: unknown;
  /** Explicit ordinary overrides; absent properties inherit. Secrets are omitted. */
  overrides: Partial<Record<"local" | "project" | "global", unknown>>;
  /** Resolution failed; value/origin/presence must not be interpreted as unset. */
  unavailable: boolean;
}

const COLUMNS = "key, value, origin, extension, doc, schema, title, category, secret, is_set, owner, restart_required, ui, local_value, project_value, global_value, unavailable";

function parsed(cell: SqlCell): unknown {
  return typeof cell === "string" ? JSON.parse(cell) : null;
}

function settingOf(row: SqlCell[]): EffectiveSetting {
  const [key, value, origin, extension, doc, schema, title, category, secret, isSet, owner, restartRequired, ui, local, project, global, unavailable] = row;
  const overrides: EffectiveSetting["overrides"] = {};
  for (const [level, value] of [["local", local], ["project", project], ["global", global]] as const) {
    if (typeof value === "string") overrides[level] = parsed(value);
  }
  return {
    key: String(key),
    value: parsed(value),
    origin: origin as SettingOrigin | null,
    extension: typeof extension === "string" ? extension : null,
    doc: typeof doc === "string" ? doc : "",
    schema: parsed(schema),
    title: typeof title === "string" ? title : String(key),
    category: typeof category === "string" ? category : "",
    secret: secret === 1,
    isSet: isSet === 1,
    owner: typeof owner === "string" ? owner : "",
    restartRequired: restartRequired === 1,
    ui: parsed(ui),
    overrides,
    unavailable: unavailable === 1,
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
