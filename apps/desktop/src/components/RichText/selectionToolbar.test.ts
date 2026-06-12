import { describe, expect, test } from "bun:test";

import { selectionToolbarVisible } from "./selectionToolbar.js";

describe("selectionToolbarVisible", () => {
  const base = {
    commentsEnabled: true,
    selectionEmpty: false,
    composerOpen: false,
    popoverOpen: false,
  };

  test("shows for a non-empty selection on a comment-enabled field", () => {
    expect(selectionToolbarVisible(base)).toBe(true);
  });

  test("hidden when the field has no comment config", () => {
    expect(selectionToolbarVisible({ ...base, commentsEnabled: false })).toBe(false);
  });

  test("hidden for a collapsed selection", () => {
    expect(selectionToolbarVisible({ ...base, selectionEmpty: true })).toBe(false);
  });

  test("hidden while the composer or a thread popover is open", () => {
    expect(selectionToolbarVisible({ ...base, composerOpen: true })).toBe(false);
    expect(selectionToolbarVisible({ ...base, popoverOpen: true })).toBe(false);
  });
});
