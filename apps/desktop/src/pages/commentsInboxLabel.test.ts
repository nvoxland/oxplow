import { expect, test } from "bun:test";

import { targetLabel } from "./CommentsInboxPage.js";

// A work item reads by its own id, whichever list holds it: the inbox
// names no list.
test("a work item's comments are labelled by the item's own id", () => {
  expect(targetLabel("work_item", "oxplow:tsk42")).toBe("tsk42");
  expect(targetLabel("work_item", "issues:ENG-12")).toBe("ENG-12");
  expect(targetLabel("file", "src/a.rs")).toBe("src/a.rs");
  expect(targetLabel("wiki", "intro")).toBe("wiki/intro");
});
