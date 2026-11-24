import { parse, stringify } from "yaml";
import type { PageColumn, PageRun, SqlCell, SqlQueryResult, VizChart, VizRun } from "../tauri-bridge/generated/bindings.js";

export const TABLE_PRESENTATIONS = ["table", "list", "number", "bar", "line", "treemap"] as const;
export type TablePresentation = typeof TABLE_PRESENTATIONS[number];
type Role = "value" | "x" | "y" | "series" | "label" | "size" | "group";

export interface TableDisplay {
  viz: TablePresentation;
  fields: Partial<Record<Role, string>>;
}

export const TABLE_DISPLAY: TableDisplay = { viz: "table", fields: {} };

export const TABLE_ROLES: Record<TablePresentation, readonly { key: Role; label: string; optional?: boolean }[]> = {
  table: [],
  list: [],
  number: [{ key: "value", label: "Value" }],
  bar: [{ key: "x", label: "Label" }, { key: "y", label: "Value" }],
  line: [{ key: "x", label: "X axis" }, { key: "y", label: "Y axis" }, { key: "series", label: "Series", optional: true }],
  treemap: [{ key: "label", label: "Label" }, { key: "size", label: "Size" }, { key: "group", label: "Group", optional: true }],
};

export function resolveTableDisplay(display: TableDisplay, { columns, rows }: SqlQueryResult): TableDisplay {
  const roles = TABLE_ROLES[display.viz];
  if (roles.length === 0 || columns.length === 0) return display;

  const numeric = (cell: SqlCell | undefined) => typeof cell === "number"
    || (typeof cell === "string" && cell.trim() !== "" && Number.isFinite(Number(cell)));
  const numbers = columns.filter((_, at) => rows.some((row) => numeric(row[at])));
  const labels = columns.filter((column) => !numbers.includes(column));
  const time = labels.find((column) => {
    const at = columns.indexOf(column);
    return rows.some((row) => {
      const cell = row[at];
      return typeof cell === "string" && /^\d{4}-\d{2}-\d{2}/.test(cell) && Number.isFinite(Date.parse(cell));
    });
  });
  const valid = (field: string | undefined) => field !== undefined && columns.includes(field) ? field : undefined;
  const x = valid(display.fields.x) ?? (display.viz === "line" ? time ?? numbers[0] : labels[0]) ?? columns[0]!;
  const y = numbers.find((column) => column !== x) ?? numbers[0] ?? columns.find((column) => column !== x) ?? x;
  const defaults: Record<Role, string> = {
    x, y, value: numbers[0] ?? columns[0]!, label: labels[0] ?? columns[0]!, size: y, series: "", group: "",
  };
  const fields = { ...display.fields };
  for (const { key } of roles) fields[key] = valid(fields[key]) ?? defaults[key];
  return { viz: display.viz, fields };
}

function chartOf(display: TableDisplay): VizChart | null {
  if (display.viz === "bar" || display.viz === "line") {
    const { x, y, series } = display.fields;
    return { x: x ?? null, y: y ?? null, ...(display.viz === "line" && series ? { series } : {}) };
  }
  if (display.viz === "treemap") {
    const { label, size, group } = display.fields;
    return { label: label ?? null, size: size ?? null, ...(group ? { group } : {}) };
  }
  return null;
}

export function displayTable(run: VizRun, display: TableDisplay): VizRun {
  if (display.viz === "table") return run;
  const result = run.result;
  const at = display.viz === "number" ? result.columns.indexOf(display.fields.value ?? "") : -1;
  const order = [at, ...result.columns.map((_, i) => i).filter((i) => i !== at)];
  return {
    ...run,
    config: { ...run.config, viz: display.viz, chart: chartOf(display) },
    result: at <= 0 ? result : {
      ...result,
      columns: order.map((i) => result.columns[i]!),
      rows: result.rows.map((row) => order.map((i) => row[i] ?? null)),
    },
  };
}

/** What a table is saved from: the run it shows, its loader, how it is
 *  shown, the params the person set (`given`: not what the page resolved,
 *  so a default like the newest commit stays a default) and its columns. */
export interface SavedTable {
  run: PageRun;
  loader: string;
  display: TableDisplay;
  title: string;
  given: Record<string, SqlCell>;
  columns: readonly PageColumn[];
  /** An inline view's YAML: its page id is temporary, so its loaders are
   *  copied. */
  inlineYaml?: string;
}

/** A column as a page's YAML writes it: only what it says. */
function writtenColumn(column: PageColumn): Record<string, unknown> {
  return Object.fromEntries(Object.entries(column).filter(([, value]) => value !== null && value !== undefined));
}

/** A saved view reads the original page loader with the inputs the person
 *  set, and keeps the table's columns (labels, formats, links). */
export function tablePageYaml({ run, loader, display, title, given, columns, inlineYaml }: SavedTable): string {
  const source: { loaders?: Record<string, unknown>; params?: { name: string; default?: SqlCell }[] } | null =
    inlineYaml === undefined ? null : parse(inlineYaml);
  const loaders: Record<string, unknown> = source === null
    ? { rows: { page: run.page.id, loader, params: given } }
    : { ...source.loaders };
  let from = source === null ? "rows" : loader;
  let written = columns.map(writtenColumn);
  if (display.viz === "number" && display.fields.value !== undefined) {
    const quote = (name: string) => `"${name.replaceAll('"', '""')}"`;
    let projection = "number_value";
    while (Object.hasOwn(loaders, projection)) projection += "_";
    loaders[projection] = { sql: `SELECT source.${quote(display.fields.value)} FROM ${quote(from)} AS source` };
    from = projection;
    written = written.filter((column) => column.key === display.fields.value);
  }
  const chart = chartOf(display);
  return stringify({
    title,
    ...(source?.params ? {
      params: source.params.map((param) => Object.hasOwn(given, param.name) ? { ...param, default: given[param.name] } : param),
    } : {}),
    ...(run.page.refresh ? { refresh: true } : {}),
    loaders,
    view: { viz: display.viz, from, ...(chart ? { chart } : {}), ...(written.length ? { columns: written } : {}) },
  });
}
