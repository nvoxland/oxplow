/**
 * Pure view-model helpers for lens pages: which columns to show, how a
 * cell links into the page graph, how values print, and the launcher
 * entries for loaded lenses. React-free so it's unit-tested directly.
 * See `.context/extensions.md`.
 */
import { formatMetricValue, formatShortDateTime } from "../components/format.js";
import type { Extension, Lens, LensChart, LensLink, LensRun, LensViz, SqlCell, SqlQueryResult } from "../tauri-bridge/generated/bindings.js";
import { PAGE_CATEGORY_ORDER, type PageDirectoryEntry } from "../components/RailHud/sections.js";
import { computeDiffId, duplicateBlockRef, effortDiffRef, refFromTabId, fileRef, gitCommitRef, lensRef, metricRef, wikiPageRef } from "../tabs/pageRefs.js";
import { WORKING, parseRevision, shortRevisionLabel } from "../revision.js";
import type { TabRef } from "../tabs/tabState.js";
import { formatContextMention, type ContextRef } from "../agent-context-ref.js";
import { parseRef } from "../refs/ref.js";

export interface DisplayColumn {
  key: string;
  label: string;
  /** Position in the result row. */
  index: number;
  link: LensLink | null;
  /** Position of the column holding this one's unit, if it names one. */
  unitIndex: number | null;
  /** Position of the column naming the cell's icon (`lensIcons.tsx`). */
  iconIndex: number | null;
  /** Position of the column naming the cell's tone. */
  toneIndex: number | null;
}

/** The columns that style rows rather than show as cells: `group.by`,
 *  `emphasis` and `depth`. */
function stylingColumns(lens: Lens): string[] {
  return [lens.group?.by, lens.emphasis, lens.depth].filter((c): c is string => !!c);
}

/** Columns to render, in order. Declared `columns` win (skipping keys the
 *  result doesn't have — `validate_extension` reports those); otherwise
 *  every result column, labelled by name. The row-styling columns are
 *  never cells. */
export function displayColumns(lens: Lens, resultColumns: string[]): DisplayColumn[] {
  const styling = stylingColumns(lens);
  const at = (key: string | null | undefined) => {
    const i = key ? resultColumns.indexOf(key) : -1;
    return i === -1 ? null : i;
  };
  if (lens.columns.length === 0) {
    return resultColumns.flatMap((key, index) =>
      styling.includes(key) ? [] : [{ key, label: key, index, link: null, unitIndex: null, iconIndex: null, toneIndex: null }],
    );
  }
  const out: DisplayColumn[] = [];
  for (const c of lens.columns) {
    const index = resultColumns.indexOf(c.key);
    if (index === -1 || styling.includes(c.key)) continue;
    out.push({
      key: c.key,
      label: c.label ?? c.key,
      index,
      link: c.link ?? null,
      unitIndex: at(c.unit),
      iconIndex: at(c.icon),
      toneIndex: at(c.tone),
    });
  }
  return out;
}

/** One `group` of a list or table: its value (as text, what a group
 *  action names), its heading, its rows, and where the heading links. */
export interface RowGroup {
  key: string;
  label: string;
  rows: SqlCell[][];
  ref: TabRef | null;
}

/** A grouped lens's rows under a heading per distinct `group.by` value,
 *  in the order each first appears; the heading links through
 *  `group.link`, read from the group's first row. Null when the lens isn't
 *  grouped or the result lacks the column. */
export function rowGroups(lens: Lens, result: SqlQueryResult): RowGroup[] | null {
  const by = lens.group?.by;
  const i = by ? result.columns.indexOf(by) : -1;
  if (!by || i === -1) return null;
  const groups = new Map<string, RowGroup>();
  for (const row of result.rows) {
    const v = row[i] ?? null;
    const key = v === null ? "" : String(v);
    const g = groups.get(key);
    if (g) {
      g.rows.push(row);
      continue;
    }
    const link = lens.group?.link ?? null;
    groups.set(key, {
      key,
      label: formatCell(v),
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
export function rowEmphasized(lens: Lens, columns: string[], row: SqlCell[]): boolean {
  const v = cellOf(columns, row, lens.emphasis);
  if (typeof v === "number") return v !== 0;
  if (typeof v === "string") return v !== "" && v !== "0" && v.toLowerCase() !== "false";
  return v === true;
}

/** How far the row is indented: its `depth` cell as a whole number
 *  (0–8), else 0. */
export function rowDepth(lens: Lens, columns: string[], row: SqlCell[]): number {
  const v = cellOf(columns, row, lens.depth);
  const n = typeof v === "number" ? v : typeof v === "string" && /^\d+$/.test(v.trim()) ? Number(v) : 0;
  return Number.isInteger(n) ? Math.max(0, Math.min(8, n)) : 0;
}

/** What dragging the row onto the agent adds to its context: the page
 *  its first link opens (a working-tree file and a wiki page as their
 *  `@` mentions, anything else as its ref), or null when it links
 *  nowhere. */
export function rowContextRef(lens: Lens, columns: string[], row: SqlCell[]): ContextRef | null {
  const ref = rowRef(lens, columns, row);
  if (!ref) return null;
  const parsed = parseRef(ref);
  if (parsed?.kind === "wiki" && parsed.rev === null) return { kind: "wiki", slug: parsed.id };
  if (parsed?.kind === "file" && parsed.rev === null && parsed.frag === null) return { kind: "file", path: parsed.id };
  return { kind: "ref", ref };
}

/** The page a linked cell opens, or null when the target value is
 *  missing. `link.from` names the column holding the target id and
 *  defaults to the cell's own column. */
export function cellLinkRef(
  link: LensLink,
  columnKey: string,
  row: SqlCell[],
  resultColumns: string[],
): TabRef | null {
  const idx = resultColumns.indexOf(link.from ?? columnKey);
  if (idx === -1) return null;
  const v = row[idx];
  if (v === null || v === undefined || v === "") return null;
  const s = String(v);
  switch (link.kind) {
    case "file": {
      const ref = fileRef(s);
      const lineIdx = link.line ? resultColumns.indexOf(link.line) : -1;
      const line = lineIdx === -1 ? null : Number(row[lineIdx]);
      return line && line > 0 ? { ...ref, payload: { path: s, version: WORKING, line } } : ref;
    }
    case "commit":
      return gitCommitRef(s);
    case "metric":
      return metricRef(s);
    case "page":
      return refFromTabId(s);
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
      return { id: computeDiffId(spec), kind: "diff", payload: spec };
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
      // A lens row carries the effort's row id (`7`) or its id (`eff7`);
      // the page is the canonical `effort:eff7`.
      const n = effortRowId(s);
      return n === null ? null : effortDiffRef(`eff${n}`);
    }
  }
}

/** Lens runs split for a page of sections (tsk1036): the ones to show, and
 *  the ones with nothing to show — no rows, no error, not a grid (whose
 *  children decide) — folded into one "Nothing found" line. */
export function foldEmpty<T extends { run: LensRun | null; error: string | null }>(
  runs: T[],
): { shown: T[]; empty: T[] } {
  const isEmpty = (r: T) => r.error === null && r.run !== null && r.run.lens.viz !== "grid" && r.run.result.rows.length === 0;
  return { shown: runs.filter((r) => !isEmpty(r)), empty: runs.filter(isEmpty) };
}

/** A stored timestamp (ISO-8601 with a time and a zone), not a bare date. */
const TIMESTAMP = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})$/;

/** Whether `v` is a stored timestamp — shown in local time (tsk1038). */
export function isTimestamp(v: SqlCell): v is string {
  return typeof v === "string" && TIMESTAMP.test(v);
}

/** Plain-text rendering of one cell: a timestamp in local time, as the rest
 *  of the app shows times; a number with its `unit` (another cell's value,
 *  `LensColumn.unit`) as a metric value (tsk1038). */
export function formatCell(v: SqlCell, unit?: SqlCell): string {
  if (v === null) return "—";
  if (typeof v === "boolean") return v ? "yes" : "no";
  if (typeof v === "number") return typeof unit === "string" ? formatMetricValue(v, unit) : String(v);
  if (isTimestamp(v)) return formatShortDateTime(v);
  return v;
}

/** Launcher entries for every loaded lens that isn't `hidden`, under its
 *  `launcher.category` (default "Lenses"). Broken or disabled extensions
 *  contribute nothing here (their errors surface in Settings). */
export function lensDirectoryEntries(extensions: Extension[]): PageDirectoryEntry[] {
  const out: PageDirectoryEntry[] = [];
  for (const ext of extensions) {
    if (!ext.enabled) continue;
    for (const lens of ext.lenses) {
      if (lens.hidden) continue;
      const ref = lensRef(lens.id);
      out.push({
        id: ref.id,
        label: lens.title,
        ref,
        category: lens.launcherCategory ?? "Lenses",
        keywords: `lens ${ext.name} ${lens.slug} ${lens.description}`,
      });
    }
  }
  return out;
}

/** The launcher directory: static pages plus lens entries, each lens
 *  placed after the static pages of its category, categories in
 *  `PAGE_CATEGORY_ORDER`, so every heading stays contiguous. */
export function mergeDirectory(
  staticPages: PageDirectoryEntry[],
  lensPages: PageDirectoryEntry[],
): PageDirectoryEntry[] {
  const all = [...staticPages, ...lensPages];
  const rank = (c: string) => {
    const i = (PAGE_CATEGORY_ORDER as readonly string[]).indexOf(c);
    return i === -1 ? PAGE_CATEGORY_ORDER.length : i;
  };
  // Stable sort: within a category, static pages keep their order and come first.
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

/** The subset of `values` that differs from the lens's defaults — what
 *  `run_lens` needs as overrides and what an agent mention should carry. */
export function changedParams(lens: Lens, values: Record<string, SqlCell>): Record<string, SqlCell> {
  const out: Record<string, SqlCell> = {};
  for (const p of lens.params) {
    if (!(p.name in values)) continue;
    const v = values[p.name] ?? null;
    if (v !== (p.default ?? null)) out[p.name] = v;
  }
  return out;
}

/** Events that can't change what a lens shows (UI bookkeeping). */
/** Cap a result's rows for a compact view (a dashboard tile), marking it
 *  truncated when rows were dropped. Returns the input unchanged when it
 *  already fits. */
export function limitRows(result: SqlQueryResult, max: number | undefined): SqlQueryResult {
  if (max === undefined || result.rows.length <= max) return result;
  return { ...result, rows: result.rows.slice(0, max), truncated: true };
}

/** A title as a lens slug: lowercase letters, digits and single dashes
 *  (what `oxplow.lens.keep` accepts). Falls back to `lens` when nothing's left. */
export function slugify(title: string): string {
  const slug = title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug || "lens";
}

/** An unsaved query from Explore Data (or a pinned query tile), shaped as
 *  a lens so the shared `LensResultView` can render it; a chart viz names
 *  its columns in `chart`. */
export function adHocLens(query: string, viz: LensViz, chart: LensChart | null = null): Lens {
  return {
    id: "explore/ad-hoc",
    extension: "explore",
    slug: "ad-hoc",
    title: "Query",
    description: "",
    query,
    viz,
    params: [],
    columns: [],
    empty: "No rows.",
    chart,
    tree: null,
    timeline: null,
    steps: null,
    hunks: null,
    form: null,
    custom: null,
    group: null,
    emphasis: null,
    depth: null,
    children: [],
    launcherCategory: null,
    hidden: false,
    actions: [],
    alert: null,
    needs: [],
    path: "",
  };
}

/** One lens row as an add-to-agent-context mention:
 *  `[oxplow lens <id> row: col=value, …] `. */
export function rowMention(lensId: string, columns: string[], row: SqlCell[]): string {
  const fields = columns.map((c, i) => `${c}=${JSON.stringify(row[i] ?? null)}`).join(", ");
  return `[oxplow lens ${lensId} row: ${fields}] `;
}

/** The first canonical ref a lens row links to, if any: what the row is
 *  about (Ask About This, the row's extension commands). */
export function rowRef(lens: Lens, columns: string[], row: SqlCell[]): string | null {
  for (const c of lens.columns) {
    const ref = c.link ? cellLinkRef(c.link, c.key, row, columns) : null;
    if (ref && parseRef(ref.id)) return ref.id;
  }
  return null;
}

/** Ask About This on a lens row (P6.D1): the first ref the row links to
 *  (`[oxplow ref commit:abc]`), or — a row that links nowhere — the row
 *  itself as a lens mention. */
export function rowAsk(lens: Lens, columns: string[], row: SqlCell[]): string {
  const ref = rowRef(lens, columns, row);
  return ref ? formatContextMention({ kind: "ref", ref }) : rowMention(lens.id, columns, row);
}

/** What a slot runs: each mounted lens with the slot params it declares
 *  (a slot offers several, e.g. `effort.review.details`'s `effort_id` and
 *  `change_id`; `run_lens` rejects undeclared ones). */
export function slotRuns(
  extensions: Extension[],
  slot: string,
  params: Record<string, SqlCell>,
  /** Only this extension's mounts (the settings slot's per-extension sections). */
  extension?: string,
): { id: string; params: Record<string, SqlCell> }[] {
  const out: { id: string; params: Record<string, SqlCell> }[] = [];
  for (const ext of extensions) {
    if (!ext.enabled) continue;
    if (extension !== undefined && ext.name !== extension) continue;
    for (const s of ext.ui.slots) {
      if (s.slot !== slot) continue;
      const lens = ext.lenses.find((l) => l.id === s.lensId);
      if (lens) out.push({ id: s.lensId, params: childParams(lens, params) });
    }
  }
  return out;
}

/** Enabled extensions that mount something into `slot`, in order. */
export function slotExtensions(extensions: Extension[], slot: string): string[] {
  return extensions.filter((e) => e.enabled && e.ui.slots.some((s) => s.slot === slot)).map((e) => e.name);
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

/** One `tree` node: its row's label, the row, and its children. */
export interface TreeNode {
  label: string;
  row: SqlCell[];
  children: TreeNode[];
}

/** `tree` viz: each row under the row whose `tree.id` its `tree.parent`
 *  names; a row with no parent, or one not in the result, is a root. A
 *  cycle is broken at the first row seen twice. */
export function treeNodes(lens: Lens, result: SqlQueryResult): TreeNode[] {
  const ids = columnValues(result, lens.tree?.id);
  const parents = columnValues(result, lens.tree?.parent);
  const labels = columnValues(result, lens.tree?.label);
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
    return { label: formatCell(labels[at] ?? null), row: result.rows[at]!, children };
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

/** `timeline` viz: each row's time, label and ref (a canonical ref, when
 *  `timeline.ref` names a column and the row has one), oldest first. */
export function timelineEntries(
  lens: Lens,
  result: SqlQueryResult,
): { at: string; label: string; ref: string | null; row: SqlCell[] }[] {
  const ats = columnValues(result, lens.timeline?.at);
  const labels = columnValues(result, lens.timeline?.label);
  if (!ats || !labels) return [];
  const refs = columnValues(result, lens.timeline?.ref);
  return ats
    .map((at, i) => ({
      at: formatCell(at),
      label: formatCell(labels[i] ?? null),
      ref: refs && refs[i] !== null && refs[i] !== undefined && refs[i] !== "" ? String(refs[i]) : null,
      row: result.rows[i]!,
    }))
    .sort((a, b) => (a.at < b.at ? -1 : a.at > b.at ? 1 : 0));
}

export type StepStatus = "done" | "active" | "failed" | "pending";

/** `steps` viz: each step's label and status, in row order (a status
 *  other than done/active/failed is pending). */
export function stepItems(lens: Lens, result: SqlQueryResult): { label: string; status: StepStatus; row: SqlCell[] }[] {
  const labels = columnValues(result, lens.steps?.label);
  if (!labels) return [];
  const statuses = columnValues(result, lens.steps?.status);
  return labels.map((l, i) => {
    const s = statuses?.[i];
    const status: StepStatus = s === "done" || s === "active" || s === "failed" ? s : "pending";
    return { label: formatCell(l), status, row: result.rows[i] ?? [] };
  });
}

/** `hunks` viz: each row's file and the two revisions it diffs. Rows
 *  whose path or revisions are missing drop out. */
export function hunkRows(lens: Lens, result: SqlQueryResult): { path: string; from: string; to: string; row: SqlCell[] }[] {
  const paths = columnValues(result, lens.hunks?.path);
  const froms = columnValues(result, lens.hunks?.from);
  const tos = columnValues(result, lens.hunks?.to);
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

/** `bar` viz rows: `chart.x` labels and `chart.y` values (NULL → 0). */
export function barRows(lens: Lens, result: SqlQueryResult): { label: string; value: number }[] {
  const xs = columnValues(result, lens.chart?.x);
  const ys = columnValues(result, lens.chart?.y);
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
  lens: Lens,
  result: SqlQueryResult,
): { name: string; points: { t: number; v: number }[] }[] {
  const xs = columnValues(result, lens.chart?.x);
  const ys = columnValues(result, lens.chart?.y);
  if (!xs || !ys) return [];
  const names = columnValues(result, lens.chart?.series);
  const bySeries = new Map<string, { t: number; v: number }[]>();
  xs.forEach((x, i) => {
    const t = toTime(x);
    const v = Number(ys[i]);
    if (t === null || ys[i] === null || Number.isNaN(v)) return;
    const name = names ? formatCell(names[i] ?? null) : "";
    const pts = bySeries.get(name) ?? [];
    pts.push({ t, v });
    bySeries.set(name, pts);
  });
  return [...bySeries.entries()].map(([name, points]) => ({ name, points: points.sort((a, b) => a.t - b.t) }));
}

/** `treemap` viz items: label, positive size, optional group, and the row
 *  (so a click can follow the lens's links). */
export function treemapItems(
  lens: Lens,
  result: SqlQueryResult,
): { label: string; size: number; group: string | null; row: SqlCell[] }[] {
  const labels = columnValues(result, lens.chart?.label);
  const sizes = columnValues(result, lens.chart?.size);
  if (!labels || !sizes) return [];
  const groups = columnValues(result, lens.chart?.group);
  const out: { label: string; size: number; group: string | null; row: SqlCell[] }[] = [];
  labels.forEach((l, i) => {
    const size = Number(sizes[i]);
    if (!(size > 0)) return;
    out.push({ label: formatCell(l), size, group: groups ? formatCell(groups[i] ?? null) : null, row: result.rows[i]! });
  });
  return out;
}

/** The params a `grid` passes to one child: those the child declares. */
export function childParams(child: Lens, params: Record<string, SqlCell>): Record<string, SqlCell> {
  const out: Record<string, SqlCell> = {};
  for (const p of child.params) {
    if (p.name in params) out[p.name] = params[p.name] ?? null;
  }
  return out;
}


/** A lens `oxplow.lens.keep` wrote to its own stream's worktree (`live: false`):
 *  the app shows the main worktree's lenses, so this one shows once the
 *  stream is merged. */
export const KEPT_IN_STREAM =
  "Saved in this stream's worktree. It shows in the app once the stream is merged.";

/** The starter prompt "New Lens with Your Agent…" puts in the agent's
 *  input for the person to finish and send (it's never sent for them).
 *  The agent's oxplow-extension skill takes it from there (tsk373). */
export const NEW_LENS_PROMPT =
  "Build me an oxplow lens (a page I can open with Cmd+P) that shows ";
