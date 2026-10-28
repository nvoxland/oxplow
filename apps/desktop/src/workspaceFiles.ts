/**
 * The stream's files as the app reads them: `v_workspace_file`, a live
 * dataset over the worktree (`.context/semantic-layer.md` "Live datasets").
 * One folder is one read (`WHERE parent = ?`); the whole tree is a walk,
 * so the callers that need it all (Quick Open, the link index, the panel's
 * search and status markers) read it once and follow its `reads` with
 * `useRerunOnChange` instead of re-reading per keystroke.
 */
import { querySql } from "./api.js";
import { streamRowId } from "./modelIds.js";
import type { Reads, SqlCell } from "./tauri-bridge/generated/bindings.js";

export interface WorkspaceIndexedFile {
  path: string;
  status: string | null;
}

const status = (cell: SqlCell | undefined): string | null => (typeof cell === "string" ? cell : null);

/** Every file of the stream (folders left out) with its status, and what
 *  the read read. A walk of the whole tree: read it once. */
export async function readWorkspaceIndex(streamId: string): Promise<{ files: WorkspaceIndexedFile[]; reads: Reads }> {
  const res = await querySql(
    "SELECT path, status FROM v_workspace_file WHERE stream_id = ?1 AND kind = 'file'",
    [streamRowId(streamId)],
    1_000_000,
  );
  return { files: res.rows.map((row) => ({ path: String(row[0]), status: status(row[1]) })), reads: res.reads };
}

/** Whether `path` is a folder of the stream's worktree (one folder's read:
 *  its parent's). */
export async function isWorkspaceDir(streamId: string, path: string): Promise<boolean> {
  const clean = path.replace(/\/+$/, "");
  if (clean === "") return true;
  const slash = clean.lastIndexOf("/");
  const parent = slash === -1 ? "" : clean.slice(0, slash);
  const res = await querySql(
    "SELECT 1 FROM v_workspace_file WHERE stream_id = ?1 AND parent = ?2 AND path = ?3 AND kind = 'dir'",
    [streamRowId(streamId), parent, clean],
    1,
  );
  return res.rows.length > 0;
}
