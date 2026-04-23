import type { BlameLine } from "../git/git.js";
import { BLAME_ZERO_SHA } from "../git/git.js";
import type { SnapshotStore } from "../persistence/snapshot-store.js";
import type { WorkItemEffortStore } from "../persistence/work-item-effort-store.js";

export interface LocalBlameEntry {
  line: number;
  source: "local" | "git" | "uncommitted";
  workItem?: {
    id: string;
    title: string;
    endedAt: string;
    effortId: string;
  };
  git?: {
    sha: string;
    author: string;
    authorMail: string;
    authorTime: number;
    summary: string;
  };
}

export interface LocalBlameDeps {
  effortStore: WorkItemEffortStore;
  snapshotStore: SnapshotStore;
  path: string;
  diskText: string;
  /** Injected so tests (and callers) can stub the git subprocess. */
  gitBlame: () => BlameLine[];
}

/**
 * Attribute each line of `diskText` to the oxplow work-item effort that last
 * touched it, falling back to git blame for any line the local walk can't
 * cover. See `.context/editor-and-monaco.md` "Blame overlay" for the UI
 * side and the local-blame plan for the algorithm rationale.
 *
 * Algorithm (runtime compute, no new tables):
 * 1. Load disk lines. Every line starts as "uncommitted".
 * 2. Walk closed efforts that touched this path, newest-first.
 *    - For each effort, read before/after file text from the pair
 *      (start_snapshot_id, end_snapshot_id). If either blob is missing
 *      (snapshot pruned), skip the effort — don't throw.
 *    - Diff before→after. Lines that are added-or-modified in that
 *      transition are "owned" by this effort.
 *    - Match each unclaimed current-disk line against the effort's
 *      after-text by longest-common-subsequence. If the matched line is
 *      in the effort's owned set, stamp the current line with this
 *      effort.
 * 3. Any line still unclaimed after the walk is checked against git
 *    blame. A committed sha overwrites "uncommitted" with a "git" entry.
 *    An all-zero sha keeps "uncommitted".
 */
export function computeLocalBlame(deps: LocalBlameDeps): LocalBlameEntry[] {
  const { effortStore, snapshotStore, path, diskText, gitBlame } = deps;
  const diskLines = splitLines(diskText);
  const entries: LocalBlameEntry[] = diskLines.map((_, i) => ({
    line: i + 1,
    source: "uncommitted",
  }));

  const efforts = effortStore.listEffortsForPath(path);
  for (const effort of efforts) {
    if (entries.every((e) => e.source !== "uncommitted")) break;
    if (!effort.endSnapshotId) continue;
    const afterDiff = snapshotStore.getSnapshotPairDiff(
      effort.startSnapshotId,
      effort.endSnapshotId,
      path,
    );
    const afterText = afterDiff.after;
    const beforeText = afterDiff.before;
    if (afterText == null) continue;
    const afterLines = splitLines(afterText);
    const beforeLines = beforeText == null ? [] : splitLines(beforeText);
    // Lines in afterText that are NEW vs beforeText (effort's "ownership").
    const ownedInAfter = linesAddedByB(beforeLines, afterLines);
    // Match current disk lines to afterText lines via LCS.
    const diskToAfter = matchLinesViaLcs(diskLines, afterLines);
    for (let i = 0; i < diskLines.length; i++) {
      const entry = entries[i]!;
      if (entry.source !== "uncommitted") continue;
      const afterIdx = diskToAfter[i];
      if (afterIdx == null) continue;
      if (!ownedInAfter[afterIdx]) continue;
      entry.source = "local";
      entry.workItem = {
        id: effort.workItemId,
        title: effort.title,
        endedAt: effort.endedAt,
        effortId: effort.effortId,
      };
    }
  }

  // Fall back to git blame for any line still uncommitted.
  const stillUncommitted = entries.some((e) => e.source === "uncommitted");
  if (stillUncommitted) {
    let gitLines: BlameLine[] = [];
    try {
      gitLines = gitBlame();
    } catch {
      gitLines = [];
    }
    const byLine = new Map<number, BlameLine>();
    for (const gl of gitLines) byLine.set(gl.line, gl);
    for (const entry of entries) {
      if (entry.source !== "uncommitted") continue;
      const gl = byLine.get(entry.line);
      if (!gl) continue;
      if (gl.sha === BLAME_ZERO_SHA) continue;
      entry.source = "git";
      entry.git = {
        sha: gl.sha,
        author: gl.author,
        authorMail: gl.authorMail,
        authorTime: gl.authorTime,
        summary: gl.summary,
      };
    }
  }

  return entries;
}

function splitLines(text: string): string[] {
  // Preserve the "trailing blank line" distinction: a file ending in \n
  // has an empty last element which we drop; blame is one entry per
  // displayed line.
  if (text === "") return [];
  const split = text.split("\n");
  if (split.length > 0 && split[split.length - 1] === "") split.pop();
  return split;
}

/**
 * Returns an array same length as `b`: `true` when line i of `b` is
 * "new" vs `a` (not part of the longest common subsequence). LCS
 * matching — classic O(m·n) DP. Lines >5000 on either side skip
 * diffing and mark every line of `b` as unchanged (graceful degradation).
 */
function linesAddedByB(a: string[], b: string[]): boolean[] {
  if (a.length === 0) return b.map(() => true);
  if (a.length > 5000 || b.length > 5000) return b.map(() => false);
  const dp = buildLcsTable(a, b);
  const added = new Array<boolean>(b.length).fill(true);
  let i = a.length;
  let j = b.length;
  while (i > 0 && j > 0) {
    if (a[i - 1] === b[j - 1]) {
      added[j - 1] = false;
      i--; j--;
    } else if (dp[i - 1]![j]! >= dp[i]![j - 1]!) {
      i--;
    } else {
      j--;
    }
  }
  return added;
}

/**
 * LCS-based one-to-one line matching between two arrays. Returns an
 * array same length as `a`; entry i is the matching index into `b`,
 * or null when the line isn't matched. Lines >5000 skip the diff and
 * return an all-null map — callers fall back to "no attribution".
 */
function matchLinesViaLcs(a: string[], b: string[]): Array<number | null> {
  const out = new Array<number | null>(a.length).fill(null);
  if (a.length === 0 || b.length === 0) return out;
  if (a.length > 5000 || b.length > 5000) return out;
  const dp = buildLcsTable(a, b);
  let i = a.length;
  let j = b.length;
  while (i > 0 && j > 0) {
    if (a[i - 1] === b[j - 1]) {
      out[i - 1] = j - 1;
      i--; j--;
    } else if (dp[i - 1]![j]! >= dp[i]![j - 1]!) {
      i--;
    } else {
      j--;
    }
  }
  return out;
}

function buildLcsTable(a: string[], b: string[]): Int32Array[] {
  const m = a.length;
  const n = b.length;
  const dp: Int32Array[] = new Array(m + 1);
  for (let i = 0; i <= m; i++) dp[i] = new Int32Array(n + 1);
  for (let i = 1; i <= m; i++) {
    const ai = a[i - 1];
    const row = dp[i]!;
    const prev = dp[i - 1]!;
    for (let j = 1; j <= n; j++) {
      row[j] = ai === b[j - 1]
        ? prev[j - 1]! + 1
        : Math.max(prev[j]!, row[j - 1]!);
    }
  }
  return dp;
}
