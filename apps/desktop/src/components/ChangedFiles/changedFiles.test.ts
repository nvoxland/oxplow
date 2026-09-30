import { expect, test } from "bun:test";
import { commitBase } from "./useChangedFiles.js";

test("a commit is compared against its first parent, or its own ^ for a root commit", () => {
  expect(commitBase("abc", ["p1", "p2"])).toBe("p1");
  expect(commitBase("abc", [])).toBe("abc^");
});
