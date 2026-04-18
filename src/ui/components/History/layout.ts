import type { GitLogCommit } from "../../api.js";

/**
 * One row of the rendered git-log graph. The graph is drawn row-by-row with
 * each row at a fixed height; each row covers a single commit.
 *
 * A `lane` corresponds to a vertical column in the graph. For a given row:
 *   - `incoming[k]` is the sha that was being tracked in lane k entering this
 *     row (i.e., the last lane assignment from the row above). null = empty.
 *   - `outgoing[k]` is the sha tracked in lane k after this commit is processed.
 *   - `column` is the lane where the commit's node is drawn.
 *   - `parentEdges` describes the graph edges that run from this row's node
 *     down to the next rows. `toCol` is where the parent is expected to appear
 *     as an incoming lane; if the parent sha is never visited (filtered out or
 *     beyond `limit`), `toCol` is still produced so we draw a short stub.
 *   - `fromAbove` is true if this commit's sha was already in an incoming lane
 *     (i.e., some child placed it). New branch heads are false.
 */
export interface GraphRow {
  commit: GitLogCommit;
  column: number;
  incoming: Array<string | null>;
  outgoing: Array<string | null>;
  parentEdges: Array<{ toCol: number; sha: string; missing: boolean }>;
  fromAbove: boolean;
}

export interface GraphLayout {
  rows: GraphRow[];
  totalColumns: number;
}

/**
 * Compute a DAG layout for a list of commits, given in parent-follows-child
 * order (newest first, the order `git log` produces by default). The algorithm
 * is the classic one used by `git log --graph`: maintain a stack of "active
 * lanes" (sha each column is waiting for), and when we hit a commit, reassign
 * its lane to its first parent. Extra parents spawn new lanes.
 */
export function layoutCommits(commits: GitLogCommit[]): GraphLayout {
  const shaSet = new Set(commits.map((c) => c.sha));
  let lanes: Array<string | null> = [];
  const rows: GraphRow[] = [];
  let maxCols = 0;

  for (const commit of commits) {
    let column = lanes.indexOf(commit.sha);
    const fromAbove = column !== -1;
    if (!fromAbove) {
      column = lanes.indexOf(null);
      if (column === -1) {
        column = lanes.length;
        lanes.push(null);
      }
      lanes[column] = commit.sha;
    }
    const incoming = lanes.slice();

    // Clear this commit from the lane state before placing parents.
    for (let k = 0; k < lanes.length; k++) if (lanes[k] === commit.sha) lanes[k] = null;

    const parentEdges: GraphRow["parentEdges"] = [];
    for (let pi = 0; pi < commit.parents.length; pi++) {
      const parentSha = commit.parents[pi]!.sha;
      const missing = !shaSet.has(parentSha);
      let pcol = lanes.indexOf(parentSha);
      if (pcol === -1) {
        if (pi === 0 && lanes[column] === null) {
          pcol = column;
        } else {
          pcol = lanes.indexOf(null);
          if (pcol === -1) {
            pcol = lanes.length;
            lanes.push(null);
          }
        }
        if (!missing) lanes[pcol] = parentSha;
      }
      parentEdges.push({ toCol: pcol, sha: parentSha, missing });
    }

    // Trim trailing nulls for a compact graph — lanes array isn't reused after
    // the row snapshot, so shrinking it keeps incoming/outgoing width tight.
    while (lanes.length > 0 && lanes[lanes.length - 1] === null) lanes.pop();
    const outgoing = lanes.slice();

    rows.push({ commit, column, incoming, outgoing, parentEdges, fromAbove });
    maxCols = Math.max(maxCols, incoming.length, outgoing.length);
  }

  return { rows, totalColumns: maxCols };
}
