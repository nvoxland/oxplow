/**
 * Where a ref opens (`.context/pages-and-tabs.md` "Linking between tabs"):
 * the one place that decides it, for every kind, from the registry. A
 * kind's `v_ref_kind.page` is a tab-id template (`refKinds.pageTabId`);
 * this fills it — reading the kind's `resolve` row when the template
 * names a column of it — and says what the window does with the result:
 * open a tab, select a thread or a stream, or nothing.
 *
 * What it decides is *what* opens; how (in this tab, a new one) stays the
 * click's (`RouteLink`, `useRouteDispatch`).
 */
import { querySql } from "../api.js";
import { pageColumns, pageTabId, refKindInfo } from "../refKinds.js";
import { idOf, typeOf } from "../uri/uri.js";
import { refFromTabId } from "./pageRefs.js";
import type { TabRef } from "./tabState.js";

export type RefTarget =
  | { tab: TabRef }
  | { thread: string }
  | { stream: string }
  | { unopenable: string };

/** Reads the `columns` of `view`'s row whose `ref` is `ref`; null when there is none. */
export type RowReader = (view: string, columns: string[], ref: string) => Promise<Record<string, unknown> | null>;

/** What the window does with tab id `tabId`. */
function targetOf(tabId: string): RefTarget {
  const thread = idOf(tabId, "oxplow/thread");
  if (thread !== null) return { thread };
  const stream = idOf(tabId, "oxplow/stream");
  if (stream !== null) return { stream };
  const tab = refFromTabId(tabId);
  return tab ? { tab } : { unopenable: `\`${tabId}\` isn't a page the window shows` };
}

/** Where `ref` opens when that needs nothing but the ref; null when its
 *  kind's template reads a column of its row (`refTarget` reads it). */
export function refTargetNow(ref: string): RefTarget | null {
  // Before the registry is read (a window starting), a ref opens as
  // itself, which is where every core kind but a few opens.
  // URI-SWITCH: the registry is keyed by resource name (`v_resource`).
  const kind = typeOf(ref);
  if (kind && !refKindInfo(kind)) return targetOf(ref);
  if (pageColumns(ref)) return null;
  const tabId = pageTabId(ref);
  return tabId === null ? { unopenable: noPage(ref) } : targetOf(tabId);
}

/** Where `ref` opens, reading its kind's row when the template needs it. */
export async function refTarget(ref: string, read: RowReader = readRow): Promise<RefTarget> {
  const needs = pageColumns(ref);
  if (!needs) return refTargetNow(ref) ?? { unopenable: noPage(ref) };
  const row = await read(needs.view, needs.columns, ref);
  if (!row) return { unopenable: `\`${ref}\` isn't in \`${needs.view}\`` };
  const tabId = pageTabId(ref, row);
  return tabId === null ? { unopenable: noPage(ref) } : targetOf(tabId);
}

function noPage(ref: string): string {
  return `\`${ref}\` has no page`;
}

/** The row of `view` whose `ref` is `ref`, its `columns` only. */
async function readRow(view: string, columns: string[], ref: string): Promise<Record<string, unknown> | null> {
  const quote = (name: string) => `"${name.replaceAll('"', '""')}"`;
  const res = await querySql(
    `SELECT ${columns.map(quote).join(", ")} FROM ${quote(view)} WHERE ref = ?1 LIMIT 1`,
    [ref],
    1,
  );
  const row = res.rows[0];
  if (!row) return null;
  return Object.fromEntries(res.columns.map((name, i) => [name, row[i]]));
}
