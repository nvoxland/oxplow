// Pure model logic for the diff view (`diff-view` page kind). Kept
// React-free so the endpoint resolution + header labeling can be
// unit-tested without mounting the page.

import type { DiffEndpoint } from "./tauri-bridge/generated/bindings.js";

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
