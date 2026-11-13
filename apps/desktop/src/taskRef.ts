/**
 * Task URIs on the TS side — the mirror of Rust's
 * `oxplow_domain::uri::build` task helpers (`.context/resources.md`).
 * A task's URI is `oxplow:oxplow/task?id=<id>&provider=<list>`,
 * whichever list it's on; the UI never reads meaning into the id.
 */

import { taskOf, taskUri } from "./uri/uri.js";

/** The `<provider>:<id>` of a task URI (what an element's
 *  `data-ref-id` and a comment anchor carry); `null` for anything that
 *  isn't one. */
export function taskId(ref: string): string | null {
  const item = taskOf(ref);
  return item === null ? null : `${item.provider}:${item.id}`;
}

/** The URI of the task whose `<provider>:<id>` (`taskId`) is
 *  `id`; null when it isn't one. */
export function taskUriOfId(id: string): string | null {
  const colon = id.indexOf(":");
  return colon <= 0 || colon === id.length - 1 ? null : taskUri(id.slice(0, colon), id.slice(colon + 1));
}

/** How a person names a task: its own id, as its list gives it
 *  (`tsk42`, `ENG-12`); the input when it isn't a task URI. */
export function taskLabel(ref: string): string {
  return taskOf(ref)?.id ?? ref;
}
