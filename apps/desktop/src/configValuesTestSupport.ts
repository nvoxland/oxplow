/// A `querySql` answer for `v_config_value`, from the settings a test
/// declares: what a fake of the api answers when the window reads them.
export interface FakeSetting {
  key: string;
  value: unknown;
  origin?: string | null;
  overrides?: Partial<Record<"local" | "project" | "global", unknown>>;
  secret?: boolean;
  isSet?: boolean;
  unavailable?: boolean;
}

export function configValueResult(settings: FakeSetting[]) {
  return {
    columns: ["key", "value", "origin", "extension", "doc", "schema", "title", "category", "secret", "is_set", "owner", "restart_required", "ui", "local_value", "project_value", "global_value", "unavailable"],
    rows: settings.map((s) => [s.key, s.secret || s.unavailable ? null : JSON.stringify(s.value ?? null), s.unavailable ? null : s.origin === undefined ? "default" : s.origin, null, "", null, s.key, "", Number(s.secret ?? false), s.unavailable ? 0 : Number(s.isSet ?? (s.value != null)), "core", 0, "{}",
      ...(["local", "project", "global"] as const).map(level =>
        !s.secret && Object.prototype.hasOwnProperty.call(s.overrides ?? {}, level) ? JSON.stringify(s.overrides![level] ?? null) : null),
      Number(s.unavailable ?? false)]),
    truncated: false,
    reads: { datasets: ["v_config_value"], tables: [], measures: [] },
    freshness: [],
  };
}

/** Whether `sql` reads the settings. */
export function readsSettings(sql: string): boolean {
  return sql.includes("FROM v_config_value");
}
