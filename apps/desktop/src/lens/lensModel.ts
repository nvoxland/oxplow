/**
 * Pure view-model helpers for lens pages: which columns to show, how a
 * cell links into the page graph, how values print, and the launcher
 * entries for loaded lenses. React-free so it's unit-tested directly.
 * See `.context/extensions.md`.
 */
import type { Extension, Lens, LensLink, SqlCell } from "../tauri-bridge/generated/bindings.js";
import type { PageDirectoryEntry } from "../components/RailHud/sections.js";
import { effortDiffRef, fileRef, lensRef, taskRef, wikiPageRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";

export interface DisplayColumn {
  key: string;
  label: string;
  /** Position in the result row. */
  index: number;
  link: LensLink | null;
}

/** Columns to render, in order. Declared `columns` win (skipping keys the
 *  result doesn't have — `validate_extension` reports those); otherwise
 *  every result column, labelled by name. */
export function displayColumns(lens: Lens, resultColumns: string[]): DisplayColumn[] {
  if (lens.columns.length === 0) {
    return resultColumns.map((key, index) => ({ key, label: key, index, link: null }));
  }
  const out: DisplayColumn[] = [];
  for (const c of lens.columns) {
    const index = resultColumns.indexOf(c.key);
    if (index === -1) continue;
    out.push({ key: c.key, label: c.label ?? c.key, index, link: c.link ?? null });
  }
  return out;
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
    case "task":
      return taskRef(s);
    case "file":
      return fileRef(s);
    case "wiki":
      return wikiPageRef(s);
    case "effort-diff":
      return effortDiffRef(s);
  }
}

/** Plain-text rendering of one cell. */
export function formatCell(v: SqlCell): string {
  if (v === null) return "—";
  if (typeof v === "boolean") return v ? "yes" : "no";
  if (typeof v === "number") return String(v);
  return v;
}

/** Launcher entries for every successfully loaded lens, under the
 *  "Lenses" category. Broken extensions contribute nothing here (their
 *  errors surface through `validate_extension` / Settings). */
export function lensDirectoryEntries(extensions: Extension[]): PageDirectoryEntry[] {
  const out: PageDirectoryEntry[] = [];
  for (const ext of extensions) {
    for (const lens of ext.lenses) {
      const ref = lensRef(lens.id);
      out.push({
        id: ref.id,
        label: lens.title,
        ref,
        category: "Lenses",
        keywords: `lens ${ext.name} ${lens.slug} ${lens.description}`,
      });
    }
  }
  return out;
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
const IGNORED_EVENT_KINDS = new Set(["pageVisitChanged", "usageRecorded"]);

/** Whether an oxplow event should re-run an open lens: any data event,
 *  plus file edits under `oxplow/extensions/` (the lens definition
 *  itself). Other file edits don't change semantic-layer data. */
export function shouldRerunLens(event: { kind: string; path?: unknown }): boolean {
  if (IGNORED_EVENT_KINDS.has(event.kind)) return false;
  if (event.kind === "workspaceChanged") {
    return typeof event.path === "string" && event.path.startsWith("oxplow/extensions/");
  }
  return true;
}
