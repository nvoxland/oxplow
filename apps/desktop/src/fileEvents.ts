/// What a file command did to the worktree, as the event log says it
/// (`file.renamed`, `file.deleted`): the window follows it with what holds
/// the path — open editors, the navigation target — so a rename or delete
/// from any caller (the file tree, an agent's command, a lens) is seen.
import type { SqlCell } from "./tauri-bridge/generated/bindings.js";

export type FileEvent =
  | { kind: "renamed"; from: string; to: string }
  | { kind: "deleted"; path: string };

/** The newest event's seq: where a window starts listening from. */
export const FILE_EVENTS_CURSOR_SQL = "SELECT coalesce(max(seq), 0) FROM v_event";

/** The stream's rename and delete events after a seq (`?1`, stream id `?2`). */
export const FILE_EVENTS_SQL = `SELECT seq, type, payload FROM v_event
  WHERE seq > ?1 AND stream_id = ?2 AND type IN ('file.renamed', 'file.deleted')
  ORDER BY seq`;

/** The events out of that query's rows, with the last seq read. */
export function fileEventsFromRows(rows: SqlCell[][], after: number): { events: FileEvent[]; cursor: number } {
  const events: FileEvent[] = [];
  let cursor = after;
  for (const [seq, type, payload] of rows) {
    cursor = Math.max(cursor, Number(seq));
    let body: Record<string, unknown>;
    try {
      body = JSON.parse(String(payload ?? "{}")) as Record<string, unknown>;
    } catch {
      continue;
    }
    if (type === "file.renamed" && typeof body.from === "string" && typeof body.to === "string") {
      events.push({ kind: "renamed", from: body.from, to: body.to });
    } else if (type === "file.deleted" && typeof body.path === "string") {
      events.push({ kind: "deleted", path: body.path });
    }
  }
  return { events, cursor };
}

/** Whether `path` is `root` or something inside it. */
export function isUnder(path: string, root: string): boolean {
  return path === root || path.startsWith(`${root}/`);
}

/** Where `path` is after `from` moved to `to`: itself, or under the moved
 *  directory; else unchanged. */
export function movedPath(path: string, from: string, to: string): string {
  return isUnder(path, from) ? `${to}${path.slice(from.length)}` : path;
}
