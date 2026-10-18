/// Where a stream's workspace is, as the models say it (`v_stream.head_sha`
/// and the local branch `v_branch` has checked out for it).
import { gitRevision, type Revision } from "./revision.js";
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

/** How far the stream's checked-out branch has parted from its upstream. */
export interface StreamUpstream {
  upstream: string | null;
  ahead: number;
  behind: number;
}

/** The SQL behind it: the local branch the stream has checked out. */
export const STREAM_UPSTREAM_SQL = `SELECT b.upstream, b.ahead, b.behind
  FROM v_branch b WHERE b.stream_id = ?1 AND b.kind = 'local'`;

/** The upstream out of that query's rows: none for a detached stream or a
 *  branch that tracks nothing. */
export function streamUpstreamFromRows(rows: SqlCell[][]): StreamUpstream {
  const [upstream, ahead, behind] = rows[0] ?? [null, 0, 0];
  const name = typeof upstream === "string" && upstream ? upstream : null;
  return { upstream: name, ahead: name ? Number(ahead ?? 0) : 0, behind: name ? Number(behind ?? 0) : 0 };
}
