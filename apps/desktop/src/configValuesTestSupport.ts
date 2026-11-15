/// A `querySql` answer for `v_config_value`, from the settings a test
/// declares: what a fake of the api answers when the window reads them.
export interface FakeSetting {
  key: string;
  value: unknown;
  origin?: string;
}

export function configValueResult(settings: FakeSetting[]) {
  return {
    columns: ["key", "value", "origin", "extension", "doc", "schema", "title", "category", "secret", "is_set", "owner", "restart_required", "ui"],
    rows: settings.map((s) => [s.key, JSON.stringify(s.value ?? null), s.origin ?? "default", null, "", null, s.key, "", 0, Number(s.value != null), "core", 0, "{}"]),
    truncated: false,
    reads: { datasets: ["v_config_value"], tables: [], measures: [] },
    freshness: [],
  };
}

/** Whether `sql` reads the settings. */
export function readsSettings(sql: string): boolean {
  return sql.includes("FROM v_config_value");
}
