import { refTargetNow } from "../tabs/refOpener.js";
import type { Origin, OriginItem } from "../tabs/session.js";
import type { SqlCell } from "../tauri-bridge/generated/bindings.js";
import { cellLinkRef, type DisplayColumn } from "./lensModel.js";

/**
 * The collection a table's links open from: a link column's pages, in row
 * order, so the opened page's Up and Down step through them (`Origin`,
 * `.context/pages-and-tabs.md` "Origins"). A link whose page needs a read
 * to name (`refTarget`) is left out; fewer than two pages make no
 * collection.
 */
export function linkOrigins(
  title: string,
  column: DisplayColumn,
  rows: SqlCell[][],
  resultColumns: string[],
): (row: SqlCell[]) => Origin | undefined {
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
  return (row) => {
    const index = indexOfRow.get(row);
    return index === undefined || items.length < 2 ? undefined : { title, items, index };
  };
}

function tabOf(ref: string) {
  const target = refTargetNow(ref);
  return target !== null && "tab" in target ? target.tab : null;
}
