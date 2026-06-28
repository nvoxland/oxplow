import { describe, expect, test } from "bun:test";
import {
  previousSnapshotId,
  resolveEffortEndpoints,
  resolveSnapshotEndpoints,
} from "./diffViewModel.js";

describe("previousSnapshotId", () => {
  test("returns the largest id strictly less than the target", () => {
    expect(previousSnapshotId(9, [3, 5, 9, 12])).toBe(5);
  });
  test("ignores ids >= target and unordered input", () => {
    expect(previousSnapshotId(9, [12, 9, 5, 3])).toBe(5);
  });
  test("null when the target is the first capture", () => {
    expect(previousSnapshotId(3, [3, 5, 9])).toBeNull();
  });
});

describe("resolveSnapshotEndpoints", () => {
  test("single snapshot → prev as start, N as end", () => {
    expect(resolveSnapshotEndpoints(9, 5)).toEqual({
      start: { kind: "snapshot", snapshot_id: 5 },
      end: { kind: "snapshot", snapshot_id: 9 },
      inProgress: false,
    });
  });
  test("first snapshot → null start (everything added)", () => {
    expect(resolveSnapshotEndpoints(3, null)).toEqual({
      start: null,
      end: { kind: "snapshot", snapshot_id: 3 },
      inProgress: false,
    });
  });
});

describe("resolveEffortEndpoints", () => {
  test("completed effort → snapshot↔snapshot, not in progress", () => {
    expect(resolveEffortEndpoints({ startSnapshotId: 3, endSnapshotId: 9 })).toEqual({
      start: { kind: "snapshot", snapshot_id: 3 },
      end: { kind: "snapshot", snapshot_id: 9 },
      inProgress: false,
    });
  });

  test("open effort (no end snapshot) → diffs start against the working tree", () => {
    expect(resolveEffortEndpoints({ startSnapshotId: 3, endSnapshotId: null })).toEqual({
      start: { kind: "snapshot", snapshot_id: 3 },
      end: { kind: "working" },
      inProgress: true,
    });
  });

  test("missing start snapshot → null start (everything added)", () => {
    expect(resolveEffortEndpoints({ startSnapshotId: null, endSnapshotId: 9 })).toEqual({
      start: null,
      end: { kind: "snapshot", snapshot_id: 9 },
      inProgress: false,
    });
  });
});
