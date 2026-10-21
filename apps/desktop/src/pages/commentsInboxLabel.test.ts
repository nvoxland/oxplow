import { expect, test } from "bun:test";

import { fileUri, wikiUri, workItemUri } from "../uri/uri.js";
import { targetLabel } from "./CommentsInboxPage.js";

// A work item reads by its own id, whichever list holds it: the inbox
// names no list.
test("a work item's comments are labelled by the item's own id", () => {
  expect(targetLabel(workItemUri("oxplow", "tsk42"))).toBe("tsk42");
  expect(targetLabel(workItemUri("issues", "ENG-12"))).toBe("ENG-12");
  expect(targetLabel(fileUri("src/a.rs"))).toBe("src/a.rs");
  expect(targetLabel(wikiUri("intro"))).toBe("wiki/intro");
});
