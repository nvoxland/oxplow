/// Where a stream's workspace is, as the models say it (`v_stream.head_sha`
/// and the local branch `v_branch` has checked out for it).
import { gitRevision, type Revision } from "./revision.js";
import type { StatusCounts } from "./api.js";
import type { SqlCell } from "./tauri-bridge/generated/bindings.js";

/** The stream's head: its revision (null before the first commit or the
 *  first refs scan) and its checked-out branch (null when detached). */
export interface StreamHead {
  revision: Revision | null;
  branch: string | null;
}

/** The SQL behind it: one row, `head_sha` and `branch`. */
export const STREAM_HEAD_SQL = `SELECT s.head_sha,
  (SELECT b.name FROM v_branch b WHERE b.stream_id = s.id AND b.kind = 'local') AS branch
  FROM v_stream s WHERE s.id = ?1`;

/** The head out of that query's rows (a stream that isn't there has none). */
export function streamHeadFromRows(rows: SqlCell[][]): StreamHead {
  const [sha, branch] = rows[0] ?? [null, null];
  return {
    revision: typeof sha === "string" && sha ? gitRevision(sha) : null,
    branch: typeof branch === "string" && branch ? branch : null,
  };
}

/** The SQL behind it: the stream's working change (`v_change`) and its
 *  files (`v_change_file`) grouped by status and whether git tracks them,
 *  with the conflicted count riding along — a stream with no changed
 *  files still answers with one row. */
export const STATUS_COUNTS_SQL = `SELECT c.conflicted, f.status, f.untracked, count(f.path)
  FROM v_change c LEFT JOIN v_change_file f ON f.change_id = c.id
  WHERE c.kind = 'working' AND c.stream_id = ?1
  GROUP BY f.status, f.untracked`;

/** The counts out of that query's rows. An untracked file counts as
 *  untracked, not added; git's rename detection isn't kept, so `renamed`
 *  is 0 (a rename is a deleted and an added file). */
export function statusCountsFromRows(rows: SqlCell[][]): StatusCounts {
  const counts: StatusCounts = { added: 0, modified: 0, deleted: 0, renamed: 0, untracked: 0, conflicted: 0, total: 0 };
  for (const [conflicted, status, untracked, n] of rows) {
    counts.conflicted = Number(conflicted ?? 0);
    const count = Number(n ?? 0);
    if (typeof status !== "string" || count === 0) continue;
    counts.total += count;
    if (Number(untracked ?? 0) === 1) counts.untracked += count;
    else if (status === "added" || status === "modified" || status === "deleted") counts[status] += count;
    else counts.modified += count;
  }
  return counts;
}
