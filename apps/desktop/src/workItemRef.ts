/**
 * Work-item refs on the TS side — the mirror of Rust's
 * `oxplow_domain::refs::build` work-item helpers (`.context/refs.md`).
 *
 * An effort is on a WORK ITEM: an oxplow task (`work_item:oxplow:tsk42`)
 * or another provider's item (`work_item:issues:ENG-12`). Only the first
 * has a task page; everything else is shown by its label.
 */

const OXPLOW_TASK = /^work_item:oxplow:(tsk\d+)$/;

/** The `work_item` ref of an oxplow task id (`tsk42`). */
export function workItemRef(taskId: string): string {
  return `work_item:oxplow:${taskId}`;
}

/** The oxplow task (`tsk42`) a work-item ref names; `null` for another
 *  provider's item or anything that isn't a work-item ref. */
export function taskIdOfWorkItemRef(ref: string): string | null {
  return OXPLOW_TASK.exec(ref)?.[1] ?? null;
}

/** How a person names a work item: `tsk42` for an oxplow task, the
 *  provider-scoped id (`issues:ENG-12`) otherwise. */
export function workItemLabel(ref: string): string {
  const task = taskIdOfWorkItemRef(ref);
  if (task) return task;
  return ref.startsWith("work_item:") && ref.length > "work_item:".length
    ? ref.slice("work_item:".length)
    : ref;
}
