/**
 * Work-item refs on the TS side — the mirror of Rust's
 * `oxplow_domain::refs::build` work-item helpers (`.context/refs.md`).
 * A work item's ref is `work_item:<provider>:<id>`, whichever list it's
 * on (`work_item:oxplow:tsk42`, `work_item:issues:ENG-12`); the UI never
 * reads meaning into the id.
 */

const PREFIX = "work_item:";

/** The `<provider>:<id>` part of a work-item ref (its `page_ref` id);
 *  `null` for anything that isn't one. */
export function workItemId(ref: string): string | null {
  if (!ref.startsWith(PREFIX)) return null;
  const rest = ref.slice(PREFIX.length);
  return rest.includes(":") && !rest.startsWith(":") && !rest.endsWith(":") ? rest : null;
}

/** How a person names a work item: its own id, as its list gives it
 *  (`tsk42`, `ENG-12`); the input when it isn't a work-item ref. */
export function workItemLabel(ref: string): string {
  const id = workItemId(ref);
  return id === null ? ref : id.slice(id.indexOf(":") + 1);
}
