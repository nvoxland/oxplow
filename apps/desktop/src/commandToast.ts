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

/** The thing a command's result names (its `uri`), which a toast offers to
 *  open; null when the result names none. */
export function resultUri(result: unknown): string | null {
  const uri = (result as { uri?: unknown } | null | undefined)?.uri;
  return typeof uri === "string" && uri !== "" ? uri : null;
}

/** The commit a command's result names (`rev`), when it made one. */
function committedRev(result: unknown): string | null {
  const rev = (result as { rev?: unknown } | null | undefined)?.rev;
  return typeof rev === "string" && rev !== "" ? rev : null;
}

/** The toast for a commit: which one. */
export function committedMessage(rev: string): string {
  return `✓ Committed ${rev.slice(0, 7)}`;
}

/** The toast for a run, by its label; a run that made a commit names it. */
export function ranMessage(label: string, result: unknown): string {
  const rev = committedRev(result);
  return rev === null ? doneMessage(label) : committedMessage(rev);
}

/** The toast for a command taken back with Undo; a commit's says where the
 *  changes went. */
export function undoneMessage(label: string, result?: unknown): string {
  return committedRev(result) === null
    ? `Undone: ${plain(label)}`
    : "Commit undone: the changes are back in the working tree";
}
