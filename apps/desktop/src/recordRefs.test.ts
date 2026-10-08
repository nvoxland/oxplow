import { expect, test } from "bun:test";

import { commentRef, effortRef, streamRef, threadRef } from "./recordRefs.js";

// A command names a record by its ref, never its bare id.

test("each record kind is named by its ref", () => {
  expect(threadRef("thr3")).toBe("thread:thr3");
  expect(streamRef("str1")).toBe("stream:str1");
  expect(effortRef("eff12")).toBe("effort:eff12");
  expect(commentRef("cmt4")).toBe("comment:cmt4");
});
