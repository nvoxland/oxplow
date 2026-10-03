import { describe, expect, test } from "bun:test";
import {
  changedExtensions,
  pickerBranch,
  previousSnapshotId,
  rangeDateLabel,
  rangeEndpointOptions,
  inProgressNotice,
  resolveEffortEndpoints,
  resolveSnapshotEndpoints,
  resolveTurnEndpoints,
  snapshotsOnBranch,
} from "./diffViewModel.js";

describe("rangeDateLabel", () => {
  test("a single date when both endpoints fall on the same calendar day", () => {
    const label = rangeDateLabel("2026-06-29T12:00:00", "2026-06-29T18:00:00");
    expect(label).not.toBeNull();
    expect(label).not.toContain("–");
  });
  test("a start – end range when the endpoints span multiple days", () => {
    const label = rangeDateLabel("2026-06-28T12:00:00", "2026-06-29T12:00:00");
    expect(label).toContain("–");
  });
  test("the single known date when only one endpoint is time-based", () => {
    expect(rangeDateLabel(null, "2026-06-29T12:00:00")).not.toContain("–");
    expect(rangeDateLabel("2026-06-29T12:00:00", null)).not.toContain("–");
  });
  test("null when neither endpoint is time-based", () => {
    expect(rangeDateLabel(null, null)).toBeNull();
  });
});

describe("rangeEndpointOptions", () => {
  const snaps = [{ id: 1 }, { id: 2 }, { id: 3 }, { id: 4 }, { id: 5 }];

  test("end options keep only snapshots after the start bound", () => {
    expect(rangeEndpointOptions(snaps, "end", 3, 5, 10).map((s) => s.id)).toEqual([5, 4]);
  });
  test("start options keep only snapshots before the end bound", () => {
    expect(rangeEndpointOptions(snaps, "start", 3, 1, 10).map((s) => s.id)).toEqual([2, 1]);
  });
  test("a null bound (other endpoint isn't a snapshot) applies no constraint", () => {
    expect(rangeEndpointOptions(snaps, "end", null, 2, 10).map((s) => s.id)).toEqual([
      5, 4, 3, 2, 1,
    ]);
  });
  test("the current selection is always kept, even outside the newest-N window", () => {
    const ids = rangeEndpointOptions(snaps, "end", null, 1, 2).map((s) => s.id);
    expect(ids).toEqual([5, 4, 1]);
  });
});

describe("pickerBranch", () => {
  test("prefers the end snapshot's branch", () => {
    expect(
      pickerBranch({ branch: "feature" }, { branch: "main" }, "other"),
    ).toBe("feature");
  });
  test("falls back to the start snapshot, then the stream branch", () => {
    expect(pickerBranch(undefined, { branch: "main" }, "other")).toBe("main");
    expect(pickerBranch({ branch: null }, { branch: null }, "other")).toBe("other");
  });
  test("null when nothing is known", () => {
    expect(pickerBranch(undefined, undefined, null)).toBeNull();
  });
});

describe("snapshotsOnBranch", () => {
  const rows = [
    { id: 1, branch: "main" },
    { id: 2, branch: "feature" },
    { id: 3, branch: null },
  ];
  test("excludes known other-branch snapshots but keeps unknown (null) ones", () => {
    // null branch is unknown, not provably-different, so it's kept — this
    // is what keeps the picker populated on pre-V42 snapshots.
    expect(snapshotsOnBranch(rows, "main").map((r) => r.id)).toEqual([1, 3]);
  });
  test("a null reference branch (unknown) disables filtering entirely", () => {
    expect(snapshotsOnBranch(rows, null)).toHaveLength(3);
  });
});

describe("previousSnapshotId", () => {
  const rows = (pairs: Array<[number, number | null]>) =>
    pairs.map(([id, parentSnapshotId]) => ({ id, parentSnapshotId }));
  test("the recorded parent wins over id order (P2.11)", () => {
    // 9 grew from 3 (5 was another take that 9's op didn't follow).
    expect(previousSnapshotId(9, rows([[3, null], [5, 3], [9, 3], [12, 9]]))).toBe(3);
  });
  test("a snapshot no op created falls back to the largest id before it", () => {
    expect(previousSnapshotId(9, rows([[12, null], [9, null], [5, null], [3, null]]))).toBe(5);
  });
  test("null when the target is the first capture", () => {
    expect(previousSnapshotId(3, rows([[3, null], [5, 3], [9, 5]]))).toBeNull();
  });
});

describe("resolveSnapshotEndpoints", () => {
  test("single snapshot → prev as start, N as end", () => {
    expect(resolveSnapshotEndpoints(9, 5)).toEqual({
      start: "snap:5",
      end: "snap:9",
      inProgress: false,
    });
  });
  test("first snapshot → null start (everything added)", () => {
    expect(resolveSnapshotEndpoints(3, null)).toEqual({
      start: null,
      end: "snap:3",
      inProgress: false,
    });
  });
});

describe("resolveEffortEndpoints", () => {
  test("completed effort → snapshot↔snapshot, not in progress", () => {
    expect(resolveEffortEndpoints({ startSnapshotId: 3, endSnapshotId: 9 })).toEqual({
      start: "snap:3",
      end: "snap:9",
      inProgress: false,
    });
  });

  test("open effort (no end snapshot) → diffs start against the working tree", () => {
    expect(resolveEffortEndpoints({ startSnapshotId: 3, endSnapshotId: null })).toEqual({
      start: "snap:3",
      end: "working",
      inProgress: true,
    });
  });

  test("missing start snapshot → null start (everything added)", () => {
    expect(resolveEffortEndpoints({ startSnapshotId: null, endSnapshotId: 9 })).toEqual({
      start: null,
      end: "snap:9",
      inProgress: false,
    });
  });
});

describe("resolveTurnEndpoints", () => {
  test("a finished turn diffs its start snapshot to its end snapshot", () => {
    expect(resolveTurnEndpoints({ startSnapshotId: 3, snapshotId: 5 })).toEqual({
      start: "snap:3",
      end: "snap:5",
      inProgress: false,
    });
  });
  test("a running turn diffs its start against the working tree", () => {
    expect(resolveTurnEndpoints({ startSnapshotId: 3, snapshotId: null })).toMatchObject({
      end: "working",
      inProgress: true,
    });
  });
  test("a turn with no start snapshot has nothing to diff against", () => {
    const r = resolveTurnEndpoints({ startSnapshotId: null, snapshotId: 5 });
    expect("unavailable" in r && r.unavailable).toContain("no snapshot");
  });
});

describe("inProgressNotice", () => {
  test("names what is still running", () => {
    expect(inProgressNotice("turn")).toContain("Turn is still running");
    expect(inProgressNotice("effort")).toContain("Effort is in progress");
    expect(inProgressNotice("endpoints")).not.toMatch(/effort|turn/i);
  });
});

test("the extensions a change touches, by folder", () => {
  expect(
    changedExtensions([
      "oxplow/extensions/b/lenses/x.yaml",
      "src/a.rs",
      "oxplow/extensions/a/extension.yaml",
      "oxplow/extensions/b/models/y.sql",
      "oxplow/extensions.yaml",
    ]),
  ).toEqual(["a", "b"]);
});
