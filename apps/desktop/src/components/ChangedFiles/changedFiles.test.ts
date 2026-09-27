import { expect, test } from "bun:test";
import { commitBase, endpointVersion } from "./useChangedFiles.js";

test("a diff endpoint maps to the file version the diff pane reads", () => {
  expect(endpointVersion({ kind: "snapshot", snapshot_id: 7 })).toEqual({ kind: "snapshot", id: "7" });
  expect(endpointVersion({ kind: "commit", sha: "abc" })).toEqual({ kind: "ref", ref: "abc" });
  expect(endpointVersion({ kind: "working" })).toEqual({ kind: "disk" });
});

test("a commit is compared against its first parent, or its own ^ for a root commit", () => {
  expect(commitBase("abc", ["p1", "p2"])).toBe("p1");
  expect(commitBase("abc", [])).toBe("abc^");
});
