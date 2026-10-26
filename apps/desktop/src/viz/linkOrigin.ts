import { refTargetNow } from "../tabs/refOpener.js";
import type { Origin, OriginItem } from "../tabs/session.js";
import type { VizRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { cellLinkRef, displayColumns, type DisplayColumn } from "./vizModel.js";

/**
 * The collection a table's links open from: a link column's pages, so the
 * opened page's Up and Down step through them (`Origin`,
 * `.context/pages-and-tabs.md` "Origins"). It is live: it names the node and
 * its params, and the host runs it again for the list (`resolveLiveOrigin`),
 * so the list is never a snapshot. Fewer than two pages make no collection.
 */
export function linkOrigins(
  title: string,
  source: { node: string; params: Record<string, SqlCell> },
  column: DisplayColumn,
  rows: SqlCell[][],
  resultColumns: string[],
): (row: SqlCell[]) => Origin | undefined {
  const { items, indexOfRow } = originItems(column, rows, resultColumns);
  const live = { ...source, column: column.key };
  return (row) => (indexOfRow.has(row) && items.length >= 2 ? { title, items: [], index: 0, live } : undefined);
}

/** The collection `origin` names, run now: its viz's link column's pages,
 *  in row order. A page that needs a read to name (`refTarget`) is left out. */
export function resolveLiveOrigin(run: VizRun, column: string): OriginItem[] {
  const shown = displayColumns(run.config, run.result.columns).find((c) => c.key === column);
  return shown ? originItems(shown, run.result.rows, run.result.columns).items : [];
}

function originItems(column: DisplayColumn, rows: SqlCell[][], resultColumns: string[]) {
  const items: OriginItem[] = [];
  const indexOfRow = new Map<SqlCell[], number>();
  for (const row of rows) {
    const target = column.link ? cellLinkRef(column.link, column.key, row, resultColumns) : null;
    if (target === null) continue;
    const tab = typeof target === "string" ? tabOf(target) : target;
    if (!tab) continue;
    indexOfRow.set(row, items.length);
    items.push({ ref: tab, label: String(row[column.index] ?? tab.id) });
  }
  return { items, indexOfRow };
}

function tabOf(ref: string) {
  const target = refTargetNow(ref);
  return target !== null && "tab" in target ? target.tab : null;
}
