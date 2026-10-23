/**
 * The stream's files as the app reads them: `v_workspace_file`, a live
 * model over the worktree (`.context/semantic-layer.md` "Live models").
 * One folder is one read (`WHERE parent = ?`); the whole tree is a walk,
 * so the callers that need it all (Quick Open, the link index, the panel's
 * search and status markers) read it once and follow its `reads` with
 * `useRerunOnChange` instead of re-reading per keystroke.
 */
import { querySql } from "./api.js";
import { streamRowId } from "./modelIds.js";
import type { FileStatus, Reads, SqlCell } from "./tauri-bridge/generated/bindings.js";

export interface WorkspaceEntry {
  name: string;
  path: string;
  kind: "file" | "directory";
  status: FileStatus | null;
  /** A file with a status, or a folder holding one (see [`withFolderChanges`]). */
  hasChanges: boolean;
}

export interface WorkspaceIndexedFile {
  path: string;
  status: FileStatus | null;
}

const status = (cell: SqlCell | undefined): FileStatus | null => (typeof cell === "string" ? (cell as FileStatus) : null);

/** One folder's entries (`path` "" is the root) and what the read read. */
export async function readWorkspaceFolder(streamId: string, path: string): Promise<{ entries: WorkspaceEntry[]; reads: Reads }> {
  const res = await querySql(
    "SELECT name, path, kind, status FROM v_workspace_file WHERE stream_id = ?1 AND parent = ?2",
    [streamRowId(streamId), path.replace(/\/+$/, "")],
    100_000,
  );
  const entries = res.rows.map((row): WorkspaceEntry => {
    const s = status(row[3]);
    return { name: String(row[0]), path: String(row[1]), kind: row[2] === "dir" ? "directory" : "file", status: s, hasChanges: s !== null };
  });
  return { entries, reads: res.reads };
}

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

/** `entries` with each folder's `hasChanges` set from the files that have
 *  a status (every ancestor of one holds a change). */
export function withFolderChanges(entries: WorkspaceEntry[], files: readonly WorkspaceIndexedFile[]): WorkspaceEntry[] {
  const folders = new Set<string>();
  for (const f of files) {
    if (f.status === null) continue;
    for (let slash = f.path.lastIndexOf("/"); slash > 0; slash = f.path.lastIndexOf("/", slash - 1)) {
      folders.add(f.path.slice(0, slash));
    }
  }
  return entries.map((e) => (e.kind === "directory" ? { ...e, hasChanges: folders.has(e.path) } : e));
}
