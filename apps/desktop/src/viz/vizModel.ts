/**
 * Pure view-model helpers for viz pages: which columns to show, how a
 * cell links into the page graph, how values print, and the launcher
 * entries for loaded viz nodes. React-free so it's unit-tested directly.
 * See `.context/extensions-pages.md`.
 */
import { humanize } from "../kit/humanize.js";
import type { FormSource } from "./formSource.js";
import type { VizActionRunner } from "./vizActionRunner.js";
import {
  formatBytes,
  formatDateOnly,
  formatDurationMs,
  formatInteger,
  formatMetricValue,
  formatPercent,
  formatRelative,
  formatShortDateTime,
} from "../components/format.js";
import type { ColumnFormat, VizConfig, PageParam, ColumnLink, VizRun, SqlCell, SqlQueryResult } from "../tauri-bridge/generated/bindings.js";
import { PAGE_CATEGORY_ORDER, type PageDirectoryEntry } from "../components/RailHud/sections.js";
import { diffRef, duplicateBlockRef, effortDiffRef, fileRefAt, gitCommitRef, metricRef, wikiPageRef } from "../tabs/pageRefs.js";
import { WORKING, parseRevision, shortRevisionLabel } from "../revision.js";
import type { TabRef } from "../tabs/tabState.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import type { AssistantContext } from "../components/Assistant/assistantApi.js";
import { idOf, parseUri } from "../uri/uri.js";

export interface DisplayColumn {
  key: string;
  label: string;
  /** Position in the result row. */
  index: number;
  link: ColumnLink | null;
  /** Position of the column holding this one's unit, if it names one. */
  unitIndex: number | null;
  /** Position of the column naming the cell's icon (`vizIcons.tsx`). */
  iconIndex: number | null;
  /** Position of the column naming the cell's tone. */
  toneIndex: number | null;
  /** Its width in pixels, when the viz fixes one. */
  width: number | null;
  /** How its value reads (`PageColumn.format`). */
  format: ColumnFormat | null;
  /** The param whose current value is marked in the cell's text. */
  highlight: string | null;
  /** The viz this column's cell opens under its row (`PageColumn.expand`). */
  expand?: string | null;
  /** Position of the column that must be truthy for a detail to draw this
   *  fact (`PageColumn.when`). */
  whenIndex: number | null;
}

/** `text` split around each case-insensitive occurrence of `needle`, the
 *  occurrences marked. An empty needle marks nothing. */
export function markParts(text: string, needle: string): { text: string; mark: boolean }[] {
  if (needle === "") return [{ text, mark: false }];
  const lower = text.toLowerCase();
  const find = needle.toLowerCase();
  const parts: { text: string; mark: boolean }[] = [];
  let from = 0;
  for (let at = lower.indexOf(find); at !== -1; at = lower.indexOf(find, from)) {
    if (at > from) parts.push({ text: text.slice(from, at), mark: false });
    parts.push({ text: text.slice(at, at + find.length), mark: true });
    from = at + find.length;
  }
  if (from < text.length || parts.length === 0) parts.push({ text: text.slice(from), mark: false });
  return parts;
}

/** The columns that style rows rather than show as cells: `group.by`,
 *  `emphasis` and `depth`. */
function stylingColumns(viz: VizConfig): string[] {
  // A `sections` viz's role columns are its heading and body, never facts.
  const s = viz.viz === "sections" ? viz.sections : null;
  return [viz.group?.by, viz.emphasis, viz.depth, s?.title, s?.id, s?.link, s?.body, s?.group, s?.group_link].filter((c): c is string => !!c);
}

/** Columns to render, in order. Declared `columns` win (skipping keys the
 *  result doesn't have — `validate_extension` reports those); otherwise
 *  every result column, labelled by name. The row-styling columns are
 *  never cells. */
export function displayColumns(viz: VizConfig, resultColumns: string[]): DisplayColumn[] {
  const styling = stylingColumns(viz);
  const at = (key: string | null | undefined) => {
    const i = key ? resultColumns.indexOf(key) : -1;
    return i === -1 ? null : i;
  };
  if (viz.columns.length === 0) {
    return resultColumns.flatMap((key, index) =>
      styling.includes(key) ? [] : [{ key, label: humanize(key), index, link: null, unitIndex: null, iconIndex: null, toneIndex: null, width: null, format: null, highlight: null, whenIndex: null }],
    );
  }
  const out: DisplayColumn[] = [];
  for (const c of viz.columns) {
    const index = resultColumns.indexOf(c.key);
    if (index === -1 || styling.includes(c.key)) continue;
    out.push({
      key: c.key,
      label: c.label ?? humanize(c.key),
      index,
      link: c.link ?? null,
      unitIndex: at(c.unit),
      iconIndex: at(c.icon),
      toneIndex: at(c.tone),
      width: c.width ?? null,
      format: c.format ?? null,
      highlight: c.highlight ?? null,
      ...(c.expand ? { expand: c.expand } : {}),
      whenIndex: at(c.when),
    });
  }
  return out;
}

/** One `group` of a list or table: its value (as text, what a group
 *  action names), its heading, its rows, and where the heading links. */
export interface RowGroup {
  key: string;
  /** The `by` value it groups (what a drop in it binds as `{{group.value}}`). */
  value: SqlCell;
  label: string;
  rows: SqlCell[][];
  ref: LinkTarget | null;
}

/** A grouped viz's rows under a heading per distinct `group.by` value:
 *  its `group.values` first, in order and shown even with no rows (an
 *  empty board column is somewhere to drop), then the rest in the order
 *  each first appears; the heading links through `group.link`, read from
 *  the group's first row. Null when the viz isn't grouped or the result
 *  lacks the column. */
export function rowGroups(viz: VizConfig, result: SqlQueryResult): RowGroup[] | null {
  const by = viz.group?.by;
  const i = by ? result.columns.indexOf(by) : -1;
  if (!by || i === -1) return null;
  const labelOf = (v: SqlCell): string => (v !== null && viz.group?.labels?.[String(v)]) || formatCell(v);
  const groups = new Map<string, RowGroup>();
  for (const v of viz.group?.values ?? []) {
    groups.set(v, { key: v, value: v, label: labelOf(v), rows: [], ref: null });
  }
  for (const row of result.rows) {
    const v = row[i] ?? null;
    const key = v === null ? "" : String(v);
    const link = viz.group?.link ?? null;
    const g = groups.get(key);
    if (g) {
      if (g.rows.length === 0 && link) g.ref = cellLinkRef(link, by, row, result.columns);
      g.rows.push(row);
      continue;
    }
    groups.set(key, {
      key,
      value: v,
      label: labelOf(v),
      rows: [row],
      ref: link ? cellLinkRef(link, by, row, result.columns) : null,
    });
  }
  return [...groups.values()];
}

function cellOf(columns: string[], row: SqlCell[], key: string | null | undefined): SqlCell {
  const i = key ? columns.indexOf(key) : -1;
  return i === -1 ? null : (row[i] ?? null);
}

/** Whether the row is highlighted: its `emphasis` cell is truthy (true,
 *  a non-zero number, text other than empty / `0` / `false`). */
export function rowEmphasized(viz: VizConfig, columns: string[], row: SqlCell[]): boolean {
  const v = cellOf(columns, row, viz.emphasis);
  if (typeof v === "number") return v !== 0;
  if (typeof v === "string") return v !== "" && v !== "0" && v.toLowerCase() !== "false";
  return v === true;
}

/** How far the row is indented: its `depth` cell as a whole number
 *  (0–8), else 0. */
export function rowDepth(viz: VizConfig, columns: string[], row: SqlCell[]): number {
  const v = cellOf(columns, row, viz.depth);
  const n = typeof v === "number" ? v : typeof v === "string" && /^\d+$/.test(v.trim()) ? Number(v) : 0;
  return Number.isInteger(n) ? Math.max(0, Math.min(8, n)) : 0;
}

/** The rows of a `depth` viz that are children of a row in `closed`
 *  (named by `idOf`), however deep, and which rows have children at all:
 *  a row is a parent when the row after it is deeper. */
export function foldedAway(
  viz: VizConfig,
  columns: string[],
  rows: SqlCell[][],
  closed: ReadonlySet<string>,
  idOf: (row: SqlCell[]) => string,
): { hidden: Set<SqlCell[]>; parents: Set<SqlCell[]> } {
  const hidden = new Set<SqlCell[]>();
  const parents = new Set<SqlCell[]>();
  if (!viz.depth) return { hidden, parents };
  let below: number | null = null;
  rows.forEach((row, i) => {
    const d = rowDepth(viz, columns, row);
    const next = rows[i + 1];
    if (next && rowDepth(viz, columns, next) > d) parents.add(row);
    if (below !== null && d > below) {
      hidden.add(row);
      return;
    }
    below = parents.has(row) && closed.has(idOf(row)) ? d : null;
  });
  return { hidden, parents };
}

/** Where a linked cell opens: a page built from the row (a file at a
 *  line, a diff between two revisions), or a ref string, which opens
 *  where its kind's page says (`openRef`). */
export type LinkTarget = TabRef | string;

/** The ref a target is, for what keys on one (badges, Ask About This). */
export function linkTargetId(target: LinkTarget): string {
  return typeof target === "string" ? target : target.id;
}

/** Where a linked cell opens, or null when the target value is
 *  missing. `link.from` names the column holding the target id and
 *  defaults to the cell's own column. */
export function cellLinkRef(
  link: ColumnLink,
  columnKey: string,
  row: SqlCell[],
  resultColumns: string[],
): LinkTarget | null {
  const idx = resultColumns.indexOf(link.from ?? columnKey);
  if (idx === -1) return null;
  const v = row[idx];
  if (v === null || v === undefined || v === "") return null;
  const s = String(v);
  switch (link.kind) {
    case "file": {
      const lineIdx = link.line ? resultColumns.indexOf(link.line) : -1;
      return fileRefAt(s, lineIdx === -1 ? null : Number(row[lineIdx]));
    }
    case "commit":
      return gitCommitRef(s);
    case "metric":
      return metricRef(s);
    case "page":
      // Any ref: the registry says where its kind opens.
      return parseUri(s) ? s : null;
    case "diff-at": {
      const at = (col: string | null) => {
        const i = col ? resultColumns.indexOf(col) : -1;
        return i === -1 ? null : (row[i] ?? null);
      };
      const base = parseRevision(at(link.base));
      const head = parseRevision(at(link.head));
      if (!base || !head) return null;
      const line = Number(at(link.line));
      const spec = {
        path: s,
        leftVersion: base,
        rightVersion: head,
        baseLabel: shortRevisionLabel(base),
        labelOverride: `${shortRevisionLabel(base)}..${shortRevisionLabel(head)}`,
        revealLine: line > 0 ? line : undefined,
      };
      return diffRef(spec);
    }
    case "compare": {
      const m = /^(.+):(\d+)-(\d+)\|(.+):(\d+)-(\d+)$/.exec(s);
      if (!m) return null;
      const headIdx = link.head ? resultColumns.indexOf(link.head) : -1;
      const version = headIdx === -1 ? WORKING : (parseRevision(row[headIdx] ?? null) ?? WORKING);
      return duplicateBlockRef({
        leftPath: m[1]!,
        leftStart: Number(m[2]),
        leftEnd: Number(m[3]),
        leftVersion: version,
        rightPath: m[4]!,
        rightStart: Number(m[5]),
        rightEnd: Number(m[6]),
        rightVersion: version,
      });
    }
    case "wiki":
      return wikiPageRef(s);
    case "effort-diff": {
      // A viz row carries the effort's ref (`v_effort.ref`); a cell that
      // isn't one links nowhere.
      const id = idOf(s, "oxplow/effort");
      return id === null ? null : effortDiffRef(id);
    }
  }
}

/** A stored timestamp (ISO-8601 with a time and a zone), not a bare date. */
const TIMESTAMP = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})$/;

/** Whether `v` is a stored timestamp — shown in local time (tsk1038). */
export function isTimestamp(v: SqlCell): v is string {
  return typeof v === "string" && TIMESTAMP.test(v);
}

/** Plain-text rendering of one cell: by its column's `format` when it
 *  has one; else a timestamp in local time, as the rest of the app shows
 *  times, and a number with its `unit` (another cell's value,
 *  `PageColumn.unit`) as a metric value (tsk1038). */
export function formatCell(v: SqlCell, unit?: SqlCell, format?: ColumnFormat | null): string {
  if (v === null) return "—";
  if (typeof v === "boolean") return v ? "yes" : "no";
  const formatted = format ? byFormat(v, format) : null;
  if (formatted !== null) return formatted;
  if (typeof v === "number") return typeof unit === "string" ? formatMetricValue(v, unit) : String(v);
  if (isTimestamp(v)) return formatShortDateTime(v);
  return v;
}

/** A `sparkline` cell's numbers: a JSON array's numbers (numeric text
 *  too; anything else in it is dropped); null when the cell isn't an array. */
export function sparklineValues(v: SqlCell | undefined): number[] | null {
  if (typeof v !== "string") return null;
  try {
    const parsed: unknown = JSON.parse(v);
    if (!Array.isArray(parsed)) return null;
    return parsed.flatMap((x) => {
      const n = typeof x === "number" ? x : typeof x === "string" && x.trim() !== "" ? Number(x) : Number.NaN;
      return Number.isFinite(n) ? [n] : [];
    });
  } catch {
    return null;
  }
}

/** One item of an `items` cell. */
export interface Item {
  text: string;
  /** Its hover text. */
  hint?: string;
  /** The page it links to. */
  uri?: string;
}

/** An `items` cell's items: a JSON array whose elements are a string or
 *  `{ text, hint?, uri? }` (anything else in it is dropped); null when the
 *  cell isn't such an array. */
export function itemList(v: SqlCell | undefined): Item[] | null {
  if (typeof v !== "string") return null;
  try {
    const parsed: unknown = JSON.parse(v);
    if (!Array.isArray(parsed)) return null;
    return parsed.flatMap((x): Item[] => {
      if (typeof x === "string") return x === "" ? [] : [{ text: x }];
      if (typeof x !== "object" || x === null) return [];
      const o = x as Record<string, unknown>;
      if (typeof o.text !== "string" || o.text === "") return [];
      const item: Item = { text: o.text };
      if (typeof o.hint === "string" && o.hint !== "") item.hint = o.hint;
      if (typeof o.uri === "string" && o.uri !== "") item.uri = o.uri;
      return [item];
    });
  } catch {
    return null;
  }
}

/** How a table column's cells wrap. `nowrap`: a date, a number, a value
 *  with a unit, or short values with no spaces stay on one line.
 *  `words`: breaks only between words, so the column is at least its
 *  longest word wide. `breakable`: holds long unbroken tokens (a URI, a
 *  path) that break anywhere, so it is the column that gives way. */
export type ColumnWrap = "nowrap" | "words" | "breakable";

const NOWRAP_FORMATS: ReadonlySet<ColumnFormat> = new Set(["date", "datetime", "relative", "bytes", "duration", "integer", "percent", "badge"]);
const SHORT_VALUE = 20;
const LONGEST_WORD = 24;

/** The wrap of a column from its `format`, whether it has a `unit`, and the
 *  texts its cells show. */
export function columnWrap(format: ColumnFormat | null, hasUnit: boolean, values: readonly string[]): ColumnWrap {
  if ((format !== null && NOWRAP_FORMATS.has(format)) || hasUnit) return "nowrap";
  const shown = values.filter((v) => v !== "");
  if (shown.length > 0 && shown.every((v) => v.length <= SHORT_VALUE && !/\s/.test(v))) return "nowrap";
  const longest = Math.max(0, ...shown.flatMap((v) => v.split(/\s+/).map((w) => w.length)));
  return longest > LONGEST_WORD ? "breakable" : "words";
}

/** `v` read by `format`; null when it isn't the kind of value the format
 *  reads (the cell then shows as it would without one). */
function byFormat(v: string | number, format: ColumnFormat): string | null {
  if (format === "sparkline") {
    const values = sparklineValues(v);
    return values === null ? null : values.length === 0 ? "" : String(values[values.length - 1]);
  }
  if (format === "items") {
    const list = itemList(v);
    return list === null ? null : list.map((i) => i.text).join(" · ");
  }
  if (typeof v === "string") {
    if (Number.isNaN(Date.parse(v))) return null;
    if (format === "date") return formatDateOnly(v);
    if (format === "datetime") return formatShortDateTime(v);
    if (format === "relative") return formatRelative(v);
    return null;
  }
  if (format === "bytes") return formatBytes(v);
  if (format === "duration") return formatDurationMs(v);
  if (format === "integer") return formatInteger(v);
  if (format === "percent") return formatPercent(v);
  return null;
}

/** The launcher directory: two lists of entries, each of `then`'s
 *  placed after `first`'s of its category, categories in
 *  `PAGE_CATEGORY_ORDER`, so every heading stays contiguous. */
export function mergeDirectory(
  first: PageDirectoryEntry[],
  then: PageDirectoryEntry[],
): PageDirectoryEntry[] {
  const all = [...first, ...then];
  const rank = (c: string) => {
    const i = (PAGE_CATEGORY_ORDER as readonly string[]).indexOf(c);
    return i === -1 ? PAGE_CATEGORY_ORDER.length : i;
  };
  // Stable sort: within a category, `first` keeps its order and comes first.
  return all
    .map((p, i) => ({ p, i }))
    .sort((a, b) => rank(a.p.category) - rank(b.p.category) || a.i - b.i)
    .map((x) => x.p);
}

/** Turn a param input box's text into a bound value: numeric text binds
 *  as a number (so `:id = 3` matches integer columns), blank as NULL,
 *  anything else as text. */
export function parseParamInput(text: string): SqlCell {
  const t = text.trim();
  if (t === "") return null;
  if (/^-?\d+(\.\d+)?$/.test(t)) return Number(t);
  return t;
}

/** The subset of `values` that differs from the viz's defaults — what
 *  `oxplow_run_page` needs as overrides and what an agent mention should carry. */
export function changedParams(viz: VizConfig, values: Record<string, SqlCell>): Record<string, SqlCell> {
  const out: Record<string, SqlCell> = {};
  for (const [name, value] of Object.entries(values)) {
    const param = viz.params.find((p) => p.name === name);
    if (!param || !isDefaultValue(param, value ?? null)) out[name] = value ?? null;
  }
  return out;
}

/** Whether `value` is `param`'s default: a bool by truth (its echo is
 *  0/1, a ref's `true`/`false`), anything else by its text. */
export function isDefaultValue(param: Pick<PageParam, "type" | "options" | "default">, value: SqlCell): boolean {
  const fallback = param.default ?? null;
  const type = param.type ?? ((param.options?.length ?? 0) > 0 ? "choice" : "text");
  if (type === "bool") return truthy(value) === truthy(fallback);
  return text(value) === text(fallback);
}

function truthy(value: SqlCell): boolean {
  return value === true || (typeof value === "number" && value !== 0) || (typeof value === "string" && value !== "" && value !== "0" && value !== "false");
}

function text(value: SqlCell): string {
  return value === null || value === undefined ? "" : String(value);
}

/** Events that can't change what a viz shows (UI bookkeeping). */
/** Cap a result's rows for a compact view (a dashboard tile), marking it
 *  truncated when rows were dropped. Returns the input unchanged when it
 *  already fits. */
export function limitRows(result: SqlQueryResult, max: number | undefined): SqlQueryResult {
  if (max === undefined || result.rows.length <= max) return result;
  return { ...result, rows: result.rows.slice(0, max), truncated: true };
}

/** A title as a viz slug: lowercase letters, digits and single dashes
 *  (what `oxplow.viz.keep` accepts). Falls back to `viz` when nothing's left. */
export function slugify(title: string): string {
  const slug = title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug || "viz";
}

/** The first canonical ref a viz row links to, if any: what the row is
 *  about (Ask About This, the row's extension commands). */
export function rowRef(viz: VizConfig, columns: string[], row: SqlCell[]): string | null {
  for (const c of viz.columns) {
    const ref = c.link ? cellLinkRef(c.link, c.key, row, columns) : null;
    if (ref && parseUri(linkTargetId(ref))) return linkTargetId(ref);
  }
  return null;
}

/** One viz row as an add-to-agent-context mention, the viz named by its
 *  page node (`<page>#<node>`): `[oxplow page <id> row: col=value, …] `. */
export function rowMention(id: string, columns: string[], row: SqlCell[]): string {
  const fields = columns.map((c, i) => `${c}=${JSON.stringify(row[i] ?? null)}`).join(", ");
  return `[oxplow page ${id} row: ${fields}] `;
}

/** Ask the Assistant about a viz row: the first page the row links to,
 *  or — a row that links nowhere — the row itself as text (a viz mention). */
export function rowAssistantContext(viz: VizConfig, columns: string[], row: SqlCell[]): AssistantContext {
  const c = rowContext(viz, columns, row);
  return "uri" in c ? { kind: "page", uri: c.uri, label: null } : { kind: "selection", uri: null, text: c.text, label: null };
}

/** Add to Context on a viz row: the first page the row links to, else
 *  the row itself as text (a viz mention). */
export function rowContext(viz: VizConfig, columns: string[], row: SqlCell[]): { uri: string } | { text: string } {
  const ref = rowRef(viz, columns, row);
  return ref ? { uri: ref } : { text: rowMention(viz.id, columns, row).trim() };
}

/** The numeric row id the `v_*` views use, from a UI effort id like
 *  `eff262` (or a bare number). */
export function effortRowId(effortId: string): number | null {
  const m = /^(?:eff)?(\d+)$/.exec(effortId);
  return m ? Number(m[1]) : null;
}

/** The numeric row id from any prefixed UI id (`tsk42`, `thr7`, `eff262`,
 *  or a bare number). */
export function numericRowId(id: string): number | null {
  const m = /^[a-z]*(\d+)$/.exec(id);
  return m ? Number(m[1]) : null;
}

function columnValues(result: SqlQueryResult, column: string | null | undefined): SqlCell[] | null {
  if (!column) return null;
  const i = result.columns.indexOf(column);
  return i === -1 ? null : result.rows.map((r) => r[i] ?? null);
}

/** One `tree` node: its row's label, the row, its `tree.id` as text, and
 *  its children. `branch`: it opens (it has children, or a lazy tree's
 *  `branch` column marks it); `lazy`: its children come from the viz run
 *  again for it (`tree.lazy`), and `state` is how that run stands while
 *  it is open: loading, or failed. */
export interface TreeNode {
  label: string;
  row: SqlCell[];
  id: string | null;
  branch: boolean;
  lazy: boolean;
  state: "loading" | { error: string } | null;
  children: TreeNode[];
}

/** `tree` viz: each row under the row whose `tree.id` its `tree.parent`
 *  names; a row with no parent, or one not in the result, is a root. A
 *  cycle is broken at the first row seen twice. */
export function treeNodes(viz: VizConfig, result: SqlQueryResult): TreeNode[] {
  const ids = columnValues(result, viz.tree?.id);
  const parents = columnValues(result, viz.tree?.parent);
  const labels = columnValues(result, viz.tree?.label);
  if (!ids || !parents || !labels) return [];
  const key = (v: SqlCell) => (v === null ? null : String(v));
  const idIndex = new Map<string, number>();
  ids.forEach((id, i) => {
    const k = key(id);
    if (k !== null && !idIndex.has(k)) idIndex.set(k, i);
  });
  const parentOf = parents.map((p) => {
    const k = key(p);
    return k === null ? null : idIndex.get(k) ?? null;
  });
  const seen = new Set<number>();
  const build = (at: number): TreeNode | null => {
    if (seen.has(at)) return null;
    seen.add(at);
    const children: TreeNode[] = [];
    parentOf.forEach((p, i) => {
      if (p === at) {
        const child = build(i);
        if (child) children.push(child);
      }
    });
    return { label: formatCell(labels[at] ?? null), row: result.rows[at]!, id: key(ids[at] ?? null), branch: children.length > 0, lazy: false, state: null, children };
  };
  const roots: TreeNode[] = [];
  parentOf.forEach((p, i) => {
    if (p === null) {
      const node = build(i);
      if (node) roots.push(node);
    }
  });
  return roots;
}

/** A lazy tree's run for one open branch: the viz run again with
 *  `tree.lazy` set to the branch's id (`useLazyBranches`). */
export interface BranchRun {
  run: VizRun | null;
  error: string | null;
  /** Where its rows' actions press and their forms start, when not where
   *  the root's do (a page's branch is a run of its own). */
  runner?: VizActionRunner;
  forms?: FormSource;
}

/** A cell that counts as true: not null, zero, empty or false. */
function truthyCell(v: SqlCell | undefined): boolean {
  return v !== null && v !== undefined && v !== 0 && v !== "" && v !== false && v !== "0" && v !== "false";
}

/** A lazy tree (`tree.lazy`): the result is one level, nested as
 *  `treeNodes` nests it; a childless row the `tree.branch` column marks is a
 *  lazy branch, closed until its id is in `open`, then holding the rows
 *  of its run in `runs` (keyed by id, built the same way), `loading` until
 *  the run answers, or the run's error. */
export function lazyTreeNodes(viz: VizConfig, result: SqlQueryResult, open: ReadonlySet<string>, runs: ReadonlyMap<string, BranchRun>): TreeNode[] {
  const branchAt = viz.tree?.branch ? result.columns.indexOf(viz.tree.branch) : -1;
  const nodesOf = (res: SqlQueryResult): TreeNode[] => treeNodes(viz, res).map(mark);
  const mark = (node: TreeNode): TreeNode => {
    if (node.children.length > 0) return { ...node, children: node.children.map(mark) };
    if (branchAt === -1 || !truthyCell(node.row[branchAt]) || node.id === null) return node;
    if (!open.has(node.id)) return { ...node, branch: true, lazy: true };
    const got = runs.get(node.id);
    if (!got) return { ...node, branch: true, lazy: true, state: "loading" };
    if (!got.run) return { ...node, branch: true, lazy: true, state: { error: got.error ?? "the run failed" } };
    return { ...node, branch: true, lazy: true, children: nodesOf(got.run.result) };
  };
  return nodesOf(result);
}

/** What stops a lazy tree from opening anything: its `branch` column
 *  missing from the result (a spec the loader never checked). */
export function lazyTreeProblem(viz: VizConfig, result: SqlQueryResult): string | null {
  const branch = viz.tree?.lazy ? viz.tree.branch : null;
  if (!branch || result.columns.includes(branch)) return null;
  return `the tree's \`branch\` column \`${branch}\` isn't in the result`;
}

/** The rows a tree draws, in its order: a spliced branch's rows under it. */
export function drawnTreeRows(nodes: readonly TreeNode[]): SqlCell[][] {
  return nodes.flatMap((n) => [n.row, ...drawnTreeRows(n.children)]);
}

/** The lazy branches to run: those in `open` that are reachable, a parent
 *  at a time, through open branches whose runs are in `runs` — so a folder
 *  left open under a closed one keeps its place without running. Each
 *  with its id cell, the lazy param's value. */
export function wantedBranches(viz: VizConfig, result: SqlQueryResult, open: ReadonlySet<string>, runs: ReadonlyMap<string, BranchRun>): { id: string; value: SqlCell }[] {
  const idAt = viz.tree?.id ? result.columns.indexOf(viz.tree.id) : -1;
  const out: { id: string; value: SqlCell }[] = [];
  const walk = (nodes: readonly TreeNode[]) => {
    for (const n of nodes) {
      if (n.lazy && n.id !== null && open.has(n.id)) {
        out.push({ id: n.id, value: idAt === -1 ? n.id : (n.row[idAt] ?? n.id) });
      }
      walk(n.children);
    }
  };
  walk(lazyTreeNodes(viz, result, open, runs));
  return out;
}

/** `timeline` viz: each row's time, label and ref (a canonical ref, when
 *  `timeline.uri` names a column and the row has one), oldest first. */
export function timelineEntries(
  viz: VizConfig,
  result: SqlQueryResult,
): { at: string; label: string; ref: string | null; row: SqlCell[] }[] {
  const ats = columnValues(result, viz.timeline?.at);
  const labels = columnValues(result, viz.timeline?.label);
  if (!ats || !labels) return [];
  const refs = columnValues(result, viz.timeline?.uri);
  return ats
    .map((at, i) => ({
      at: formatCell(at),
      label: formatCell(labels[i] ?? null),
      ref: refs && refs[i] !== null && refs[i] !== undefined && refs[i] !== "" ? String(refs[i]) : null,
      row: result.rows[i]!,
    }))
    .sort((a, b) => (a.at < b.at ? -1 : a.at > b.at ? 1 : 0));
}

/** One subsection of a `sections` viz: a row. */
export interface SectionRow {
  /** Stable per row: its `id` column (`sections.id`, else a column named
   *  `id`), else its title; a repeat gets `-2`, `-3`, … so keys are unique. */
  key: string;
  title: string;
  /** The page its heading links to (`sections.link`: a URI), if any. */
  link: LinkTarget | null;
  /** Its markdown, if any. */
  body: string | null;
  /** The group it is under (`sections.group`), if it has one. */
  group: string | null;
  /** The page its group's heading opens (`sections.group_link`), if any. */
  groupLink: LinkTarget | null;
  row: SqlCell[];
}

/** What a `sections` viz draws, in order: a group (its rows under one
 *  heading, at the place its first row is) or a row with no group. */
export type SectionPart =
  | { kind: "group"; name: string; link: LinkTarget | null; rows: SectionRow[] }
  | { kind: "row"; section: SectionRow };

/** `sections` rows as the parts the viz draws: rows with the same group
 *  value together, groups in the order each first appears; the group's
 *  link is its first row's. */
export function sectionParts(sections: readonly SectionRow[]): SectionPart[] {
  const parts: SectionPart[] = [];
  const at = new Map<string, Extract<SectionPart, { kind: "group" }>>();
  for (const s of sections) {
    if (s.group === null) {
      parts.push({ kind: "row", section: s });
      continue;
    }
    const known = at.get(s.group);
    if (known) known.rows.push(s);
    else {
      const part: Extract<SectionPart, { kind: "group" }> = { kind: "group", name: s.group, link: s.groupLink, rows: [s] };
      at.set(s.group, part);
      parts.push(part);
    }
  }
  return parts;
}

/** `sections` viz: a subsection per row, in row order. A row with no
 *  title shows its key. */
export function sectionRows(viz: VizConfig, result: SqlQueryResult): SectionRow[] {
  const titles = columnValues(result, viz.sections?.title);
  if (!titles) return [];
  const keys = columnValues(result, viz.sections?.id ?? (result.columns.includes("id") ? "id" : null));
  const links = columnValues(result, viz.sections?.link);
  const bodies = columnValues(result, viz.sections?.body);
  const groups = columnValues(result, viz.sections?.group);
  const groupLinks = columnValues(result, viz.sections?.group_link);
  const seen = new Map<string, number>();
  return titles.map((t, i) => {
    const title = formatCell(t);
    const raw = keys?.[i];
    const base = raw !== null && raw !== undefined && raw !== "" ? String(raw) : title;
    const n = (seen.get(base) ?? 0) + 1;
    seen.set(base, n);
    const link = links?.[i];
    const body = bodies?.[i];
    const group = groups?.[i];
    const groupLink = groupLinks?.[i];
    return {
      key: n === 1 ? base : `${base}-${n}`,
      title: title === "" ? base : title,
      link: typeof link === "string" && link !== "" && parseUri(link) ? link : null,
      body: body === null || body === undefined || String(body).trim() === "" ? null : String(body),
      group: group === null || group === undefined || String(group) === "" ? null : String(group),
      groupLink: typeof groupLink === "string" && groupLink !== "" && parseUri(groupLink) ? groupLink : null,
      row: result.rows[i] ?? [],
    };
  });
}

export type StepStatus = "done" | "active" | "failed" | "pending";

/** `steps` viz: each step's label and status, in row order (a status
 *  other than done/active/failed is pending). */
export function stepItems(viz: VizConfig, result: SqlQueryResult): { label: string; status: StepStatus; row: SqlCell[] }[] {
  const labels = columnValues(result, viz.steps?.label);
  if (!labels) return [];
  const statuses = columnValues(result, viz.steps?.status);
  return labels.map((l, i) => {
    const s = statuses?.[i];
    const status: StepStatus = s === "done" || s === "active" || s === "failed" ? s : "pending";
    return { label: formatCell(l), status, row: result.rows[i] ?? [] };
  });
}

/** `hunks` viz: each row's file and the two revisions it diffs. Rows
 *  whose path or revisions are missing drop out. */
export function hunkRows(viz: VizConfig, result: SqlQueryResult): { path: string; from: string; to: string; row: SqlCell[] }[] {
  const paths = columnValues(result, viz.hunks?.path);
  const froms = columnValues(result, viz.hunks?.from);
  const tos = columnValues(result, viz.hunks?.to);
  if (!paths || !froms || !tos) return [];
  const out: { path: string; from: string; to: string; row: SqlCell[] }[] = [];
  paths.forEach((p, i) => {
    const from = froms[i];
    const to = tos[i];
    if (typeof p === "string" && p && typeof from === "string" && from && typeof to === "string" && to) {
      out.push({ path: p, from, to, row: result.rows[i] ?? [] });
    }
  });
  return out;
}

/** Lines `"12-40"` (or `"12"`) as a 1-based inclusive range, ends in
 *  order; null when the value isn't one (Rust's `viz_text::parse_lines`). */
export function parseLines(value: SqlCell | undefined): { start: number; end: number } | null {
  if (typeof value !== "string" && typeof value !== "number") return null;
  const text = String(value).trim();
  const [a, b = a] = text.split("-").map((p) => p.trim());
  const start = Number(a);
  const end = Number(b);
  if (!Number.isInteger(start) || !Number.isInteger(end) || start < 1 || end < 1 || !a || !b) return null;
  return { start: Math.min(start, end), end: Math.max(start, end) };
}

/** A `code` viz row: a file on a filesystem at a revision (none of
 *  either: the local working tree) and the lines it marks. */
export type CodeRow = { path: string; filesystem: string | null; revision: string | null; lines: { start: number; end: number } | null; row: SqlCell[] };

/** `code` viz: each row's file, the filesystem and revision it is read at
 *  (a column each, optional) and the lines it marks. Rows without a file
 *  are left out. */
export function codeRows(viz: VizConfig, result: SqlQueryResult): CodeRow[] {
  const paths = columnValues(result, viz.code?.path);
  if (!paths) return [];
  const filesystems = columnValues(result, viz.code?.filesystem);
  const revisions = columnValues(result, viz.code?.revision);
  const lines = columnValues(result, viz.code?.lines);
  const out: CodeRow[] = [];
  paths.forEach((p, i) => {
    if (typeof p === "string" && p) {
      out.push({ path: p, filesystem: nonEmpty(filesystems?.[i]), revision: nonEmpty(revisions?.[i]), lines: parseLines(lines?.[i]), row: result.rows[i] ?? [] });
    }
  });
  return out;
}

const nonEmpty = (v: SqlCell | undefined): string | null => (typeof v === "string" && v ? v : null);

export type MediaRow = { path: string; filesystem: string | null; revision: string | null; row: SqlCell[] };

/** `media` viz: each row's file, the filesystem and revision it is read at
 *  (a column each, optional). Rows without a file are left out. */
export function mediaRows(viz: VizConfig, result: SqlQueryResult): MediaRow[] {
  const paths = columnValues(result, viz.media?.path);
  if (!paths) return [];
  const filesystems = columnValues(result, viz.media?.filesystem);
  const revisions = columnValues(result, viz.media?.revision);
  const out: MediaRow[] = [];
  paths.forEach((p, i) => {
    if (typeof p === "string" && p) {
      out.push({ path: p, filesystem: nonEmpty(filesystems?.[i]), revision: nonEmpty(revisions?.[i]), row: result.rows[i] ?? [] });
    }
  });
  return out;
}

/** `diff` viz: the first row's file between its two revisions, landing on
 *  its `lines`' start; null when the first row doesn't name them. */
export function diffRow(viz: VizConfig, result: SqlQueryResult): DiffSpec | null {
  const at = (column: string | null | undefined) => columnValues(result, column)?.[0];
  const path = at(viz.diff?.path);
  const from = parseRevision(at(viz.diff?.from));
  const to = parseRevision(at(viz.diff?.to));
  if (typeof path !== "string" || !path || !from || !to) return null;
  const line = parseLines(at(viz.diff?.lines))?.start;
  const base = at(viz.diff?.base);
  const baseLabel = typeof base === "string" && base ? base : shortRevisionLabel(from);
  return { path, leftVersion: from, rightVersion: to, baseLabel, ...(line ? { revealLine: line } : {}) };
}

/** `editor` viz: the working tree's file the first row names and the line
 *  it lands on (its `lines`' start); null when the row names no file. */
export function editorRow(viz: VizConfig, result: SqlQueryResult): { path: string; filesystem: string | null; line: number | null } | null {
  const at = (column: string | null | undefined) => columnValues(result, column)?.[0];
  const path = at(viz.editor?.path);
  if (typeof path !== "string" || !path) return null;
  return { path, filesystem: nonEmpty(at(viz.editor?.filesystem)), line: parseLines(at(viz.editor?.lines))?.start ?? null };
}

/** `embed` viz: the web page the first row names; null when it names none. */
export function embedRow(viz: VizConfig, result: SqlQueryResult): { url: string } | null {
  const url = columnValues(result, viz.embed?.url)?.[0];
  return typeof url === "string" && url ? { url } : null;
}

/** `bar` viz rows: `chart.x` labels and `chart.y` values (NULL → 0). */
export function barRows(viz: VizConfig, result: SqlQueryResult): { label: string; value: number }[] {
  const xs = columnValues(result, viz.chart?.x);
  const ys = columnValues(result, viz.chart?.y);
  if (!xs || !ys) return [];
  return xs.map((x, i) => ({ label: formatCell(x), value: Number(ys[i] ?? 0) || 0 }));
}

/** A time or number as epoch-ms (numbers pass through). */
function toTime(v: SqlCell): number | null {
  if (typeof v === "number") return v;
  if (typeof v === "string") {
    const t = Date.parse(v);
    return Number.isNaN(t) ? null : t;
  }
  return null;
}

/** `line` viz: one series per `chart.series` value (a single unnamed
 *  series without it), points sorted by time. */
export function lineSeries(
  viz: VizConfig,
  result: SqlQueryResult,
): { name: string; points: { t: number; v: number }[]; target: number | null }[] {
  const xs = columnValues(result, viz.chart?.x);
  const ys = columnValues(result, viz.chart?.y);
  if (!xs || !ys) return [];
  const names = columnValues(result, viz.chart?.series);
  const goals = columnValues(result, viz.chart?.targets);
  const targets = new Map<string, number>();
  const bySeries = new Map<string, { t: number; v: number }[]>();
  xs.forEach((x, i) => {
    const t = toTime(x);
    const v = Number(ys[i]);
    if (t === null || ys[i] === null || Number.isNaN(v)) return;
    const name = names ? formatCell(names[i] ?? null) : "";
    const pts = bySeries.get(name) ?? [];
    pts.push({ t, v });
    bySeries.set(name, pts);
    // A series' target is the last one its rows state.
    const goal = goals?.[i];
    if (goal !== null && goal !== undefined && goal !== "" && !Number.isNaN(Number(goal))) targets.set(name, Number(goal));
  });
  return [...bySeries.entries()].map(([name, points]) => ({
    name,
    points: points.sort((a, b) => a.t - b.t),
    target: targets.get(name) ?? null,
  }));
}

/** `treemap` viz items: label, positive size, optional group, and the row
 *  (so a click can follow the viz's links). */
export function treemapItems(
  viz: VizConfig,
  result: SqlQueryResult,
): { label: string; size: number; group: string | null; row: SqlCell[] }[] {
  const labels = columnValues(result, viz.chart?.label);
  const sizes = columnValues(result, viz.chart?.size);
  if (!labels || !sizes) return [];
  const groups = columnValues(result, viz.chart?.group);
  const out: { label: string; size: number; group: string | null; row: SqlCell[] }[] = [];
  labels.forEach((l, i) => {
    const size = Number(sizes[i]);
    if (!(size > 0)) return;
    out.push({ label: formatCell(l), size, group: groups ? formatCell(groups[i] ?? null) : null, row: result.rows[i]! });
  });
  return out;
}



/** A page's sections split: the ones to show, and the ones with nothing to
 *  show — no rows, no error, not a form (which has no rows of its own) and
 *  not one with a button above its rows (an Add that works on an empty
 *  list) — folded into one "Nothing found" line. */
export function foldEmpty<T extends { run: VizRun | null; error: string | null }>(
  runs: T[],
): { shown: T[]; empty: T[] } {
  const isEmpty = (r: T) =>
    r.error === null && r.run !== null && r.run.config.viz !== "form" &&
    r.run.result.rows.length === 0 &&
    !r.run.config.actions.some((a) => !a.row && a.group === null && !a.oneRow);
  return { shown: runs.filter((r) => !isEmpty(r)), empty: runs.filter(isEmpty) };
}
