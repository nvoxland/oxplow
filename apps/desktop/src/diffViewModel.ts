// Pure model logic for the diff view (`diff-view` page kind). Kept
// React-free so the endpoint resolution + header labeling can be
// unit-tested without mounting the page.

import { formatDateOnly, isSameCalendarDay } from "./components/format.js";
import type { DiffEndpoint } from "./tauri-bridge/generated/bindings.js";

/** A date (or date range) label for the diff's range, shown above the
 *  Start/End time pickers in the rail. One date when both endpoints fall
 *  on the same calendar day; a "start – end" range when they span
 *  multiple days; the single known date when only one endpoint is
 *  time-based; null when neither is (commit↔commit / working). */
export function rangeDateLabel(
  startIso: string | null,
  endIso: string | null,
): string | null {
  if (startIso && endIso) {
    return isSameCalendarDay(startIso, endIso)
      ? formatDateOnly(startIso)
      : `${formatDateOnly(startIso)} – ${formatDateOnly(endIso)}`;
  }
  const only = startIso ?? endIso;
  return only ? formatDateOnly(only) : null;
}

export interface EffortLike {
  startSnapshotId: number | null;
  endSnapshotId: number | null;
}

export interface ResolvedEndpoints {
  start: DiffEndpoint | null;
  end: DiffEndpoint;
  /** `end` snapshot is null → the effort is still open; it diffs its
   *  start against the live working tree. The working-tree endpoint
   *  isn't computed by the substrate yet (tsk339), so the page shows an
   *  "Effort is in progress" notice rather than a file diff. */
  inProgress: boolean;
}

/** Map an effort's snapshot bracket to diff endpoints. A completed
 *  effort diffs start→end snapshots; an open effort diffs its start
 *  against the working tree. */
export function resolveEffortEndpoints(effort: EffortLike): ResolvedEndpoints {
  const start: DiffEndpoint | null =
    effort.startSnapshotId != null
      ? { kind: "snapshot", snapshot_id: effort.startSnapshotId }
      : null;
  if (effort.endSnapshotId != null) {
    return { start, end: { kind: "snapshot", snapshot_id: effort.endSnapshotId }, inProgress: false };
  }
  return { start, end: { kind: "working" }, inProgress: true };
}

/** The snapshot immediately before `snapshotId` in a stream — the
 *  largest captured id strictly less than it, or null when it's the
 *  first. A single-snapshot diff uses this as its `start`. */
export function previousSnapshotId(
  snapshotId: number,
  snapshotIds: number[],
): number | null {
  let prev: number | null = null;
  for (const id of snapshotIds) {
    if (id < snapshotId && (prev === null || id > prev)) prev = id;
  }
  return prev;
}

/** A single captured snapshot, framed as a diff: it's the `end`, and
 *  the previous snapshot in the stream is the `start` (null when it's
 *  the first capture → diff against the empty tree). */
export function resolveSnapshotEndpoints(
  snapshotId: number,
  prevSnapshotId: number | null,
): ResolvedEndpoints {
  return {
    start:
      prevSnapshotId != null
        ? { kind: "snapshot", snapshot_id: prevSnapshotId }
        : null,
    end: { kind: "snapshot", snapshot_id: snapshotId },
    inProgress: false,
  };
}

/** The branch a diff's snapshot picker should be scoped to: the branch
 *  the diffed endpoints sit on (the `end` snapshot preferred, then
 *  `start`), falling back to the stream's current branch. Null when none
 *  is known — the caller then leaves the list unfiltered. */
export function pickerBranch(
  endSnap: { gitBranch: string | null } | undefined,
  startSnap: { gitBranch: string | null } | undefined,
  streamBranch: string | null,
): string | null {
  return endSnap?.gitBranch ?? startSnap?.gitBranch ?? streamBranch ?? null;
}

/** Drop snapshots *known* to be on a different branch than `branch`, so
 *  the diff picker never mixes in another branch of the same stream's
 *  worktree. A snapshot with an unrecorded branch (null/undefined —
 *  pre-V42 rows, detached HEAD) is kept: it isn't provably different, and
 *  excluding it would empty the picker on existing snapshots. A null
 *  reference `branch` (unknown) disables filtering entirely. */
export function snapshotsOnBranch<T extends { gitBranch?: string | null }>(
  snapshots: T[],
  branch: string | null,
): T[] {
  if (branch == null) return snapshots;
  return snapshots.filter((s) => s.gitBranch == null || s.gitBranch === branch);
}

/** Options for one end of the diff range's snapshot picker, newest
 *  first, capped at `limit`. Keeps the range valid: a `"start"` picker
 *  only offers snapshots strictly *before* `bound` (the end snapshot's
 *  id), and an `"end"` picker only snapshots strictly *after* `bound`
 *  (the start's id) — so the user can't pick an inverted range. A null
 *  `bound` (the opposite endpoint isn't a snapshot — working tree /
 *  commit) means no constraint. `currentId` (this side's selected
 *  snapshot) is always kept, even when it falls outside the newest-N
 *  window or the bound. */
export function rangeEndpointOptions<T extends { id: number }>(
  snapshots: T[],
  side: "start" | "end",
  bound: number | null,
  currentId: number | null,
  limit: number,
): T[] {
  const valid = snapshots.filter((s) => {
    if (s.id === currentId) return true; // never drop the live selection
    if (bound == null) return true;
    return side === "start" ? s.id < bound : s.id > bound;
  });
  const picked = valid.sort((a, b) => b.id - a.id).slice(0, limit);
  if (currentId != null && !picked.some((s) => s.id === currentId)) {
    const cur = snapshots.find((s) => s.id === currentId);
    if (cur) picked.push(cur);
  }
  return picked.sort((a, b) => b.id - a.id);
}

/** The snapshot id pair to feed `listEffortsOverlappingRange`, or null
 *  when the range has no snapshot endpoints to overlap against. */
export function snapshotRange(
  start: DiffEndpoint | null,
  end: DiffEndpoint,
): { rangeStart: number; rangeEnd: number } | null {
  const startId = start?.kind === "snapshot" ? start.snapshot_id : null;
  const endId = end.kind === "snapshot" ? end.snapshot_id : null;
  if (endId == null) return null;
  // A null/absent start side anchors the range open at 0 so efforts
  // whose window ends at-or-before `end` still surface.
  return { rangeStart: startId ?? 0, rangeEnd: endId };
}
