import { expect, test } from "bun:test";

import { diffAskTarget } from "./diffAsk.js";

// Ask About This on a diff names the right side's file at its revision; a
// literal right side (a compare with the clipboard) has nothing to name,
// so the pane doesn't offer the action there at all.
test("a diff's right side is askable unless it's literal text", () => {
  expect(diffAskTarget({ path: "a.rs", rightVersion: "working" })).toEqual({ path: "a.rs", rev: null });
  expect(diffAskTarget({ path: "a.rs", rightVersion: "git:abc" })).toEqual({ path: "a.rs", rev: "git:abc" });
  expect(diffAskTarget({ path: "a.rs", rightVersion: "git:abc", rightContent: "pasted" })).toBeNull();
});
