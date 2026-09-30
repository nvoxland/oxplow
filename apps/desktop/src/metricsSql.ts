/// Metrics read through SQL (P4.7, `.context/semantic-layer.md` "Metrics in
/// SQL"): the pages, tiles and pickers ask `query_sql` for `metric_grid()`,
/// `v_metric_spec` and `v_metric_catalog` rather than bespoke metric IPC.
/// This module is the pure half — the queries and the row shapes; `api.ts`
/// runs them.
import { cellNumber, cellText, rowObjects } from "./sqlRows.js";
import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** One capture of a metric: its value and where it was recorded. */
export type SeriesPoint = {
  capture_id: number;
  captured_at: string;
  value: number;
  /** The dimension value, when the series is grouped. */
  group: string | null;
  branch: string | null;
  provenance: string | null;
  git_version: string | null;
  source: string | null;
};

/** A metric definition — a row of `v_metric_spec`. */
export type MetricSpec = {
  key: string;
  title: string;
  unit: string | null;
  source_measure: string | null;
  aggregation: string;
  direction: string;
  target: number | null;
  warn_at: number | null;
  fail_at: number | null;
  description: string | null;
  category: string | null;
  language: string | null;
  scope: string;
  display_kind: string;
  extension: string | null;
  entity_json: string | null;
};

/** A metric this project can use, and whether it's on — `v_metric_catalog`. */
export type MetricCatalogEntry = {
  key: string;
  title: string;
  kind: string;
  language: string | null;
  scope: string;
  enabled: boolean;
  target: number | null;
  trigger: string;
  toggleable: boolean;
  category: string | null;
};

/** A string as a SQL literal. */
function lit(s: string): string {
  return `'${s.replace(/'/g, "''")}'`;
}

/** A name as a SQL identifier. */
function ident(s: string): string {
  return `"${s.replace(/"/g, '""')}"`;
}

/** One row per capture of metric `key`, newest first, optionally grouped
 *  by a dimension and bounded to `[?1, ?2]` (RFC 3339) when `ranged`. */
export function metricSeriesSql(key: string, groupBy?: string | null, ranged = false): string {
  const measure = `MEASURE(${lit(key)})`;
  const grid = groupBy ? `metric_grid('capture', ${lit(groupBy)})` : "metric_grid('capture')";
  const group = groupBy ? `g.${ident(groupBy)}` : "NULL";
  const window = ranged ? " AND g.bucket >= ?1 AND g.bucket <= ?2" : "";
  return [
    `SELECT g.capture_id, g.bucket AS captured_at, ${measure} AS value, ${group} AS "group",`,
    "       c.branch, c.provenance, c.closest_git_version AS git_version, c.source",
    `FROM ${grid} g LEFT JOIN v_capture c ON c.id = g.capture_id`,
    `WHERE ${measure} IS NOT NULL${window}`,
    "ORDER BY g.bucket DESC",
  ].join("\n");
}

export const METRIC_SPECS_SQL =
  "SELECT key, title, unit, source_measure, aggregation, direction, target, warn_at, fail_at, " +
  "description, category, language, scope, display_kind, extension, entity_json " +
  "FROM v_metric_spec ORDER BY key";

export const METRIC_CATALOG_SQL =
  "SELECT key, title, kind, language, scope, enabled, target, trigger, toggleable, category " +
  "FROM v_metric_catalog ORDER BY key";

const text = cellText;
const num = cellNumber;

export function seriesPoints(result: SqlQueryResult): SeriesPoint[] {
  return rowObjects(result).map((r) => ({
    capture_id: num(r.capture_id) ?? 0,
    captured_at: text(r.captured_at) ?? "",
    value: num(r.value) ?? 0,
    group: text(r.group),
    branch: text(r.branch),
    provenance: text(r.provenance),
    git_version: text(r.git_version),
    source: text(r.source),
  }));
}

export function metricSpecs(result: SqlQueryResult): MetricSpec[] {
  return rowObjects(result).map((r) => ({
    key: text(r.key) ?? "",
    title: text(r.title) ?? "",
    unit: text(r.unit),
    source_measure: text(r.source_measure),
    aggregation: text(r.aggregation) ?? "",
    direction: text(r.direction) ?? "neutral",
    target: num(r.target),
    warn_at: num(r.warn_at),
    fail_at: num(r.fail_at),
    description: text(r.description),
    category: text(r.category),
    language: text(r.language),
    scope: text(r.scope) ?? "",
    display_kind: text(r.display_kind) ?? "gauge",
    extension: text(r.extension),
    entity_json: text(r.entity_json),
  }));
}

export function catalogEntries(result: SqlQueryResult): MetricCatalogEntry[] {
  return rowObjects(result).map((r) => ({
    key: text(r.key) ?? "",
    title: text(r.title) ?? "",
    kind: text(r.kind) ?? "",
    language: text(r.language),
    scope: text(r.scope) ?? "",
    enabled: r.enabled === 1 || r.enabled === true,
    target: num(r.target),
    trigger: text(r.trigger) ?? "auto",
    toggleable: r.toggleable === 1 || r.toggleable === true,
    category: text(r.category),
  }));
}
