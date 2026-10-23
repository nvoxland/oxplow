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
  vcs_rev: string | null;
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

export const METRIC_SPECS_SQL =
  "SELECT key, title, unit, source_measure, aggregation, direction, target, warn_at, fail_at, " +
  "description, category, language, scope, display_kind, extension, entity_json " +
  "FROM v_metric_spec ORDER BY key";

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
    vcs_rev: text(r.vcs_rev),
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
