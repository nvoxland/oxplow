import { describe, expect, test } from "bun:test";
import { endpointLabel, resolveEffortEndpoints } from "./diffViewModel.js";

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

describe("endpointLabel", () => {
  const commits = new Map<number, string | null>([
    [9, "abcdef1234567890"],
    [3, null],
  ]);

  test("null endpoint reads as empty", () => {
    expect(endpointLabel(null, commits)).toEqual({ text: "(empty)", commitSha: null });
  });

  test("working endpoint", () => {
    expect(endpointLabel({ kind: "working" }, commits)).toEqual({
      text: "working tree",
      commitSha: null,
    });
  });

  test("commit endpoint shows the short sha and is clickable", () => {
    expect(endpointLabel({ kind: "commit", sha: "abcdef1234567890" }, commits)).toEqual({
      text: "abcdef1",
      commitSha: "abcdef1234567890",
    });
  });

  test("snapshot with a pinned commit shows the short sha", () => {
    expect(endpointLabel({ kind: "snapshot", snapshot_id: 9 }, commits)).toEqual({
      text: "abcdef1",
      commitSha: "abcdef1234567890",
    });
  });

  test("snapshot without a commit reads as a local snapshot", () => {
    expect(endpointLabel({ kind: "snapshot", snapshot_id: 3 }, commits)).toEqual({
      text: "local snapshot 3",
      commitSha: null,
    });
  });
});
