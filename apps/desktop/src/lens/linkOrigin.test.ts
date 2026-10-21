import { expect, test } from "bun:test";

import { linkOrigins } from "./linkOrigin.js";
import type { DisplayColumn } from "./lensModel.js";

const column = { key: "subject", label: "Snapshot", index: 1, link: { kind: "page", from: "ref" } } as unknown as DisplayColumn;
const columns = ["ref", "subject"];
const rows = [
  ["snapshot:3", "third"],
  ["", "no page"],
  ["snapshot:2", "second"],
];

// A table's links are one collection: the page a row opens steps through
// the rows that have one.
test("a link column's pages are the origin, in row order, each row at its own place", () => {
  const origin = linkOrigins("Snapshots", column, rows, columns);
  const second = origin(rows[2]!)!;
  expect(second.index).toBe(1);
  expect(second.title).toBe("Snapshots");
  expect(second.items.map((i) => [i.ref.id, i.label])).toEqual([
    ["snapshot:3", "third"],
    ["snapshot:2", "second"],
  ]);
  expect(origin(rows[1]!)).toBeUndefined();
});

test("one page makes no collection", () => {
  expect(linkOrigins("Snapshots", column, [rows[0]!], columns)(rows[0]!)).toBeUndefined();
});
