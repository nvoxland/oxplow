// Explore Data's pure half: the queries it runs over oxplow's models and
// what it makes of their rows. The model registry is the catalog —
// `v_model_column` for a model's documented columns, `v_model_lineage` for
// what it reads and what reads it — and a metric reads as a line through
// `metric_grid()`. `main.ts` runs them over the bridge (`querySql`).

/** A cell of a result row. */
export type Cell = string | number | boolean | null;

/** What `querySql` answers. */
export interface Rows {
  columns: string[];
  rows: Cell[][];
  truncated: boolean;
}

/** The columns a chart viz names (a viz node's `chart:`). */
export interface Chart {
  x: string | null;
  y: string | null;
  series: string | null;
  label: string | null;
  size: string | null;
  group: string | null;
}

/** How a result is shown: a kit viz the host draws. */
export type Viz = "table" | "list" | "number" | "markdown" | "bar" | "line" | "treemap";
export const VIZ_OPTIONS: Viz[] = ["table", "list", "number", "markdown", "bar", "line", "treemap"];

/** One documented column of a model. */
export interface ModelColumn {
  name: string;
  /** What SQLite reports; empty for a computed column. */
  sqlType: string;
  doc: string;
}

/** What one model reads, and what reads it. */
export interface Lineage {
  /** Its inputs: other models (`ref`) and tables (`source`), by name. */
  reads: { name: string; kind: string }[];
  /** The models that read it. */
  readBy: string[];
}

export const MODEL_SQL = "SELECT view, owner, description FROM v_model WHERE view = ?1";
export const MODEL_COLUMNS_SQL = "SELECT name, sql_type, doc FROM v_model_column WHERE view = ?1 ORDER BY position";
export const LINEAGE_SQL = "SELECT view, input, kind FROM v_model_lineage WHERE view = ?1 OR input = ?1";
export const METRICS_SQL = "SELECT key, title FROM v_metric_catalog ORDER BY title";
/** The dimensions a metric can be sliced by (`v_dimension`). */
export const DIMENSIONS_SQL = "SELECT key, label FROM v_dimension ORDER BY label";

/** Rows a picked model's sample shows. */
export const SAMPLE_LIMIT = 50;

/** The sample query a picked model starts from (its name is the
 *  registry's: `v_work_item`). */
export function sampleSql(model: string): string {
  return `SELECT * FROM ${model} LIMIT ${SAMPLE_LIMIT}`;
}

const text = (v: Cell | undefined): string | null => (v === null || v === undefined ? null : String(v));

/** `result`'s rows as objects by column. */
export function rowObjects(result: Rows): Record<string, Cell>[] {
  return result.rows.map((row) => Object.fromEntries(result.columns.map((c, i) => [c, row[i] ?? null])));
}

export function modelColumns(result: Rows): ModelColumn[] {
  return rowObjects(result).map((r) => ({
    name: text(r.name) ?? "",
    sqlType: text(r.sql_type) ?? "",
    doc: text(r.doc) ?? "",
  }));
}

/** `view`'s lineage from the rows `LINEAGE_SQL` returned for it, sorted. */
export function lineage(view: string, result: Rows): Lineage {
  const reads: Lineage["reads"] = [];
  const readBy: string[] = [];
  for (const r of rowObjects(result)) {
    const reader = text(r.view) ?? "";
    const input = text(r.input) ?? "";
    if (reader === view) reads.push({ name: input, kind: text(r.kind) ?? "" });
    else if (input === view) readBy.push(reader);
  }
  reads.sort((a, b) => a.name.localeCompare(b.name));
  readBy.sort();
  return { reads, readBy };
}

const NO_CHART: Chart = { x: null, y: null, series: null, label: null, size: null, group: null };

/** The columns each chart viz names, and which are optional. */
export const CHART_ROLES: Partial<Record<Viz, { key: keyof Chart; label: string; optional: boolean }[]>> = {
  bar: [
    { key: "x", label: "X", optional: false },
    { key: "y", label: "Y", optional: false },
  ],
  line: [
    { key: "x", label: "X", optional: false },
    { key: "y", label: "Y", optional: false },
    { key: "series", label: "Slice By", optional: true },
  ],
  treemap: [
    { key: "label", label: "Label", optional: false },
    { key: "size", label: "Size", optional: false },
    { key: "group", label: "Group", optional: true },
  ],
};

/** A chart viz's starting columns from a result: the first column as the
 *  x / label, the first numeric one as the y / size. `null` for a viz
 *  that isn't a chart. */
export function chartDefaults(viz: Viz, result: Rows): Chart | null {
  if (viz !== "bar" && viz !== "line" && viz !== "treemap") return null;
  const first = result.columns[0] ?? null;
  const numeric =
    result.columns.find((c, i) => i > 0 && result.rows.some((r) => typeof r[i] === "number")) ?? result.columns[1] ?? null;
  return viz === "treemap" ? { ...NO_CHART, label: first, size: numeric } : { ...NO_CHART, x: first, y: numeric };
}

/** Every column `chart` names is one of `columns`. */
export function chartFits(chart: Chart, columns: string[]): boolean {
  return Object.values(chart).every((c) => c === null || columns.includes(c));
}

/** The explorer's own metric query: a metric's captures as a line, sliced
 *  by a dimension (`metric_grid`'s second argument) or not. Slice By
 *  regenerates it — free SQL is never rewritten. */
export interface MetricTemplate {
  metric: string;
  dimension: string | null;
  sql: string;
  viz: Viz;
  chart: Chart;
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
export function parseTemplate(value: string): MetricTemplate | null {
  if (value === "") return null;
  const [metric, dimension] = value.split(",");
  return metricTemplate(metric!, dimension || null);
}

/** The chart a `chart` view param says (JSON); none for text that isn't one. */
export function parseChart(value: string): Chart | null {
  if (value === "") return null;
  try {
    const parsed: unknown = JSON.parse(value);
    return parsed !== null && typeof parsed === "object" && !Array.isArray(parsed) ? (parsed as Chart) : null;
  } catch {
    return null;
  }
}

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
    "       c.branch, c.provenance, c.closest_vcs_rev AS vcs_rev, c.source",
    `FROM ${grid} g LEFT JOIN v_capture c ON c.id = g.capture_id`,
    `WHERE ${measure} IS NOT NULL${window}`,
    "ORDER BY g.bucket DESC",
  ].join("\n");
}

/** What the explorer shows: its view (the tab's address) as one value. */
export interface ExploreView {
  model: string;
  sql: string;
  viz: Viz;
  chart: Chart | null;
  template: MetricTemplate | null;
}

/** The explorer's view from its tab's view params. */
export function readView(params: Record<string, string>): ExploreView {
  const viz = (VIZ_OPTIONS as string[]).includes(params.viz ?? "") ? (params.viz as Viz) : "table";
  return {
    model: params.model ?? "",
    sql: params.sql ?? "",
    viz,
    chart: parseChart(params.chart ?? ""),
    template: parseTemplate(params.template ?? ""),
  };
}

/** The inline page the host draws for a query (`showView`): the query as
 *  one `sql` loader and its viz over it — what Save as Page keeps. JSON is
 *  YAML, so it goes as is. */
export function specOf(sql: string, viz: Viz, chart: Chart | null): string {
  return JSON.stringify({
    title: "Explore",
    data: { rows: { sql } },
    view: { id: "rows", viz, from: "rows", ...(chart ? { chart } : {}) },
  });
}
