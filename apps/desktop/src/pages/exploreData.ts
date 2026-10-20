/// Explore Data's catalog, read through SQL (P4.10, tsk517): the model
/// registry is the catalog — `v_model` for what exists, `v_model_column`
/// for each model's documented columns. The pure half; the page runs the
/// queries.
import { metricSeriesSql } from "../metricsSql.js";
import type { LensChart, LensViz } from "../tauri-bridge/generated/bindings.js";
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

/** What one model reads, and what reads it (`v_model_lineage`). */
export interface Lineage {
  /** Its inputs: other models (`ref`) and tables (`source`), by name. */
  reads: { name: string; kind: string }[];
  /** The models that read it. */
  readBy: string[];
}

export const LINEAGE_SQL = "SELECT view, input, kind FROM v_model_lineage WHERE view = ?1 OR input = ?1";

/** `view`'s lineage from the rows `LINEAGE_SQL` returned for it, sorted. */
export function lineage(view: string, result: SqlQueryResult): Lineage {
  const reads: Lineage["reads"] = [];
  const readBy: string[] = [];
  for (const r of rowObjects(result)) {
    const reader = cellText(r.view) ?? "";
    const input = cellText(r.input) ?? "";
    if (reader === view) reads.push({ name: input, kind: cellText(r.kind) ?? "" });
    else if (input === view) readBy.push(reader);
  }
  reads.sort((a, b) => a.name.localeCompare(b.name));
  readBy.sort();
  return { reads, readBy };
}

/** Why a query can't be kept as a lens or a dashboard tile, if it can't:
 *  a raw read reaches physical tables, and what's kept reads only models. */
export function keepBlockedReason(raw: boolean): string | null {
  return raw
    ? "Turn off raw tables to keep this: a lens or dashboard tile reads only models, never physical tables."
    : null;
}

// ---- Charts and Slice By (P6.F1) ----

const NO_CHART: LensChart = { x: null, y: null, series: null, label: null, size: null, group: null };

/** A chart viz's starting columns from a result: the first column as the
 *  x / label, the first numeric one as the y / size. `null` for a viz
 *  that isn't a chart. */
export function chartDefaults(viz: LensViz, result: SqlQueryResult): LensChart | null {
  if (viz !== "bar" && viz !== "line" && viz !== "treemap") return null;
  const first = result.columns[0] ?? null;
  const numeric =
    result.columns.find((c, i) => i > 0 && result.rows.some((r) => typeof r[i] === "number")) ?? result.columns[1] ?? null;
  return viz === "treemap" ? { ...NO_CHART, label: first, size: numeric } : { ...NO_CHART, x: first, y: numeric };
}

/** The explorer's own metric query: a metric's captures as a line, sliced
 *  by a dimension (`metric_grid`'s second argument) or not. Slice By
 *  regenerates it — free SQL is never rewritten. */
export interface MetricTemplate {
  metric: string;
  dimension: string | null;
  sql: string;
  viz: LensViz;
  chart: LensChart;
}

export function metricTemplate(metric: string, dimension: string | null): MetricTemplate {
  return {
    metric,
    dimension,
    sql: metricSeriesSql(metric, dimension),
    viz: "line",
    chart: { ...NO_CHART, x: "captured_at", y: "value", series: dimension ? "group" : null },
  };
}

export function sliceTemplate(template: MetricTemplate, dimension: string | null): MetricTemplate {
  return metricTemplate(template.metric, dimension);
}

/** A template as the page's `template` view param: its metric, then its
 *  dimension after a comma. */
export function templateParam(template: MetricTemplate): string {
  return template.dimension ? `${template.metric},${template.dimension}` : template.metric;
}

/** The template a `template` view param says; none for an empty one. */
export function parseTemplate(text: string): MetricTemplate | null {
  if (text === "") return null;
  const [metric, dimension] = text.split(",");
  return metricTemplate(metric!, dimension || null);
}

/** The chart a `chart` view param says (JSON); none for text that isn't one. */
export function parseChart(text: string): LensChart | null {
  if (text === "") return null;
  try {
    const parsed: unknown = JSON.parse(text);
    return parsed !== null && typeof parsed === "object" && !Array.isArray(parsed) ? (parsed as LensChart) : null;
  } catch {
    return null;
  }
}

/** The dimensions a metric can be sliced by (`v_dimension`). */
export const DIMENSIONS_SQL = "SELECT key, label FROM v_dimension ORDER BY label";
