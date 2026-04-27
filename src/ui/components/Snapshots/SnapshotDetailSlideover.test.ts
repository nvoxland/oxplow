import { describe, expect, test } from "bun:test";
import { buildSnapshotSlideoverTitle } from "./SnapshotDetailSlideover.js";

describe("buildSnapshotSlideoverTitle", () => {
  test("uses the explicit label when present", () => {
    expect(buildSnapshotSlideoverTitle({ label: "Auto saved", source: "task-end" })).toBe("Auto saved");
  });

  test("falls back to a friendly source label for known kinds", () => {
    expect(buildSnapshotSlideoverTitle({ label: null, source: "task-end" })).toBe("Task ended");
    expect(buildSnapshotSlideoverTitle({ label: null, source: "task-start" })).toBe("Task started");
    expect(buildSnapshotSlideoverTitle({ label: null, source: "startup" })).toBe("External changes");
  });

  test("uses 'Snapshot' for unknown sources", () => {
    expect(buildSnapshotSlideoverTitle({ label: null, source: "weird-thing" })).toBe("Snapshot");
  });

  test("treats blank labels as missing", () => {
    expect(buildSnapshotSlideoverTitle({ label: "   ", source: "task-end" })).toBe("Task ended");
  });
});
