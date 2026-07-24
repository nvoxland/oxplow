/**
 * Line-level working-tree-vs-HEAD diff behind the editor's gutter
 * decorations. Pure and Monaco-free (the decoration builder takes the
 * `monaco` namespace as an argument) so it can be unit-tested without
 * booting an editor — see `editor-diff.test.ts`.
 *
 * This runs synchronously inside `EditorPane`'s decoration effect, on
 * the renderer's main thread, every time a file's content changes. A
 * stall here freezes the whole app, so the walk below is written to
 * advance unconditionally; see the comment on the final `else`.
 */

export type LineKind = null | "added" | "modified";

export interface LineDiff {
  /** Per line of `newLines` (0-indexed): how it changed, or null. */
  kinds: LineKind[];
  /** Index N means: a pure deletion happened just before `newLines[N]`. */
  deletedBefore: boolean[];
}

// Line-level diff between old and new arrays. Returns, for each line in
// `newLines` (1-indexed), either "added", "modified", or null. "modified"
// is an added line that sits next to a deletion — i.e. a changed line.
// "deleted" regions are collapsed onto the next surviving line as a
// bottom marker.
export function diffLineKinds(oldLines: string[], newLines: string[]): LineDiff {
  const m = oldLines.length, n = newLines.length;
  // Guard: very large files — skip diffing to avoid quadratic blowup.
  if (m > 5000 || n > 5000) {
    return { kinds: new Array(n).fill(null), deletedBefore: new Array(n + 1).fill(false) };
  }
  const dp: Int32Array[] = new Array(m + 1);
  for (let i = 0; i <= m; i++) dp[i] = new Int32Array(n + 1);
  for (let i = 1; i <= m; i++) {
    for (let j = 1; j <= n; j++) {
      dp[i][j] = oldLines[i - 1] === newLines[j - 1]
        ? dp[i - 1][j - 1] + 1
        : Math.max(dp[i - 1][j], dp[i][j - 1]);
    }
  }
  const added = new Array<boolean>(n).fill(false);
  const deleted = new Array<boolean>(m).fill(false);
  let i = m, j = n;
  while (i > 0 && j > 0) {
    if (oldLines[i - 1] === newLines[j - 1]) { i--; j--; }
    else if (dp[i - 1][j] >= dp[i][j - 1]) { deleted[i - 1] = true; i--; }
    else { added[j - 1] = true; j--; }
  }
  while (i > 0) { deleted[i - 1] = true; i--; }
  while (j > 0) { added[j - 1] = true; j--; }

  // Walk the edit script in forward order to classify adds as
  // "modified" (accompanied by deletes in the same hunk) vs pure "added",
  // and to locate pure-deletion hunks (boundary markers).
  //
  // Driven purely by the `added`/`deleted` marks above. It must NOT
  // re-derive the alignment by comparing line values: the backtrack has
  // already chosen one particular alignment, and with duplicate lines
  // (blank lines, ``` fences — every markdown file) a value comparison
  // happily consumes a pair the backtrack marked as added/deleted. The
  // walk then drifts out of sync with the edit script until both
  // pointers land on unmarked lines that differ, where it has nothing
  // left to consume and stops advancing.
  const kinds: LineKind[] = new Array(n).fill(null);
  const deletedBefore = new Array<boolean>(n + 1).fill(false);
  let oi = 0, nj = 0;
  while (oi < m || nj < n) {
    let addedCount = 0, deletedCount = 0;
    const hunkStart = nj;
    while (nj < n && added[nj]) { addedCount++; nj++; }
    while (oi < m && deleted[oi]) { deletedCount++; oi++; }
    if (addedCount > 0) {
      const kind = deletedCount > 0 ? "modified" : "added";
      for (let k = hunkStart; k < nj; k++) kinds[k] = kind;
    } else if (deletedCount > 0) {
      // Pure deletion — mark the boundary on the surviving line.
      deletedBefore[hunkStart] = true;
    } else {
      // Neither side is marked, so both pointers sit on a line the
      // backtrack kept: consume the pair. Stepping each pointer
      // independently (rather than trusting them to be equal) is what
      // bounds this loop at m + n iterations — every iteration now
      // advances at least one pointer, so it cannot spin. That matters:
      // this runs synchronously in the decoration effect, so a stall
      // here is a frozen renderer, not a slow one.
      if (oi < m) oi++;
      if (nj < n) nj++;
    }
  }
  return { kinds, deletedBefore };
}

export function computeDiffDecorations(monaco: any, oldLines: string[], newLines: string[]): any[] {
  const { kinds, deletedBefore } = diffLineKinds(oldLines, newLines);
  const decos: any[] = [];
  for (let k = 0; k < kinds.length; k++) {
    const kind = kinds[k];
    if (!kind) continue;
    const line = k + 1;
    decos.push({
      range: new monaco.Range(line, 1, line, 1),
      options: {
        isWholeLine: true,
        linesDecorationsClassName: kind === "added" ? "oxplow-gutter-added" : "oxplow-gutter-modified",
      },
    });
  }
  // Render pure-deletion markers as a red bottom-bar on the line above
  // the missing content (or on line 1 if at the start of file).
  for (let k = 0; k < deletedBefore.length; k++) {
    if (!deletedBefore[k]) continue;
    const line = Math.max(1, Math.min(newLines.length, k));
    decos.push({
      range: new monaco.Range(line, 1, line, 1),
      options: {
        isWholeLine: true,
        linesDecorationsClassName: "oxplow-gutter-deleted",
      },
    });
  }
  return decos;
}
