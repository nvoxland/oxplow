// What a toast says after a command the person ran: one wording for the
// command menus, a page's actions and its forms.

/** A command's label as a toast names it: without the menu's decoration
 *  (a leading `+`, a trailing `…` that means "asks for more"). */
function plain(label: string): string {
  return label
    .replace(/^\s*\+\s*/, "")
    .replace(/\s*(…|\.\.\.)\s*$/, "")
    .trim();
}

/** The toast for a command that ran. Never "done": that reads as a task's
 *  state ("State: done." after moving one to in progress). */
export function doneMessage(label: string): string {
  return `✓ ${plain(label)}`;
}

/** The toast for a command taken back with Undo. */
export function undoneMessage(label: string): string {
  return `Undone: ${plain(label)}`;
}
