/**
 * Work-item URIs on the TS side — the mirror of Rust's
 * `oxplow_domain::uri::build` work-item helpers (`.context/resources.md`).
 * A work item's URI is `oxplow:oxplow/work_item?id=<id>&provider=<list>`,
 * whichever list it's on; the UI never reads meaning into the id.
 */

import { workItemOf, workItemUri } from "./uri/uri.js";

/** The `<provider>:<id>` of a work-item URI (what an element's
 *  `data-ref-id` and a comment anchor carry); `null` for anything that
 *  isn't one. */
export function workItemId(ref: string): string | null {
  const item = workItemOf(ref);
  return item === null ? null : `${item.provider}:${item.id}`;
}

/** The URI of the work item whose `<provider>:<id>` (`workItemId`) is
 *  `id`; null when it isn't one. */
export function workItemUriOfId(id: string): string | null {
  const colon = id.indexOf(":");
  return colon <= 0 || colon === id.length - 1 ? null : workItemUri(id.slice(0, colon), id.slice(colon + 1));
}

/** How a person names a work item: its own id, as its list gives it
 *  (`tsk42`, `ENG-12`); the input when it isn't a work-item URI. */
export function workItemLabel(ref: string): string {
  return workItemOf(ref)?.id ?? ref;
}
