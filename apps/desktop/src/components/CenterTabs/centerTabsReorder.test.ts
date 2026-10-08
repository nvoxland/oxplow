import { describe, expect, test } from "bun:test";
import { leadingPinnedCount, moveToIndex, reorderToAfterPinned } from "./centerTabsReorder.js";

describe("leadingPinnedCount", () => {
  test("counts the leading run of pinned tabs (the agent sessions')", () => {
    expect(leadingPinnedCount([true, false, false, false])).toBe(1);
    expect(leadingPinnedCount([false, false])).toBe(0);
    expect(leadingPinnedCount([true, true, false])).toBe(2);
  });
});

describe("reorderToAfterPinned", () => {
  const ids = ["agent", "local-history", "uncommitted", "snapshot:112", "tasks"];

  test("promotes an overflowed tab to right after the pinned agent", () => {
    expect(reorderToAfterPinned(ids, 1, "snapshot:112")).toEqual([
      "agent",
      "snapshot:112",
      "local-history",
      "uncommitted",
      "tasks",
    ]);
  });

  test("no-op when already in the slot right after pinned", () => {
    expect(reorderToAfterPinned(ids, 1, "local-history")).toBe(ids);
  });

  test("no-op for an unknown id", () => {
    expect(reorderToAfterPinned(ids, 1, "nope")).toBe(ids);
  });

  test("promotes to the front when there are no pinned tabs", () => {
    expect(reorderToAfterPinned(["a", "b", "c"], 0, "c")).toEqual(["c", "a", "b"]);
  });
});

describe("moveToIndex", () => {
  const ids = ["agent", "a", "b", "c"];
  // desiredIndex is the slot in the ORIGINAL array: "before target" =
  // targetIdx, "after target" = targetIdx + 1.
  test("drop before an earlier target (drag c before a)", () => {
    expect(moveToIndex(ids, "c", 1)).toEqual(["agent", "c", "a", "b"]);
  });
  test("drop after a later target (drag a after c)", () => {
    expect(moveToIndex(ids, "a", 4)).toEqual(["agent", "b", "c", "a"]);
  });
  test("drop before a later target (drag a before c)", () => {
    expect(moveToIndex(ids, "a", 3)).toEqual(["agent", "b", "a", "c"]);
  });
  test("no-op when it would land in place", () => {
    // a is at index 1; before b (index 2) → desiredIndex 2 → adjusted 1.
    expect(moveToIndex(ids, "a", 2)).toBe(ids);
    expect(moveToIndex(ids, "a", 1)).toBe(ids);
  });
  test("clamps and no-ops an unknown id", () => {
    expect(moveToIndex(ids, "nope", 2)).toBe(ids);
    expect(moveToIndex(ids, "c", 99)).toBe(ids); // already last
  });
});
