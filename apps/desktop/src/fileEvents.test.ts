import { expect, test } from "bun:test";

import { fileEventsFromRows, isUnder, movedPath } from "./fileEvents.js";

test("rename and delete events are read from the log's rows, in order, with the last seq", () => {
  const { events, cursor } = fileEventsFromRows(
    [
      [5, "file.renamed", JSON.stringify({ stream: "str1", from: "a.ts", to: "b.ts" })],
      [7, "file.deleted", JSON.stringify({ stream: "str1", path: "dir" })],
      [8, "file.deleted", "not json"],
    ],
    4,
  );
  expect(events).toEqual([
    { kind: "renamed", from: "a.ts", to: "b.ts" },
    { kind: "deleted", path: "dir" },
  ]);
  expect(cursor).toBe(8);
  expect(fileEventsFromRows([], 4)).toEqual({ events: [], cursor: 4 });
});

test("a path moves with its directory", () => {
  expect(movedPath("src/a.ts", "src", "lib")).toBe("lib/a.ts");
  expect(movedPath("src/a.ts", "src/a.ts", "src/b.ts")).toBe("src/b.ts");
  expect(movedPath("srcs/a.ts", "src", "lib")).toBe("srcs/a.ts");
  expect(isUnder("src/a/b", "src")).toBe(true);
  expect(isUnder("src", "src")).toBe(true);
  expect(isUnder("srcs", "src")).toBe(false);
});
