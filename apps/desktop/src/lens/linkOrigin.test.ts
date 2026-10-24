import { expect, test } from "bun:test";

import { linkOrigins, resolveLiveOrigin } from "./linkOrigin.js";
import type { DisplayColumn } from "./lensModel.js";

const column = { key: "subject", label: "Snapshot", index: 1, link: { kind: "page", from: "uri" } } as unknown as DisplayColumn;
const columns = ["uri", "subject"];
const source = { lens: "x/snaps", params: { n: 3 } };
const rows = [
  ["oxplow:oxplow/snapshot?id=3", "third"],
  ["", "no page"],
  ["oxplow:oxplow/snapshot?id=2", "second"],
];

// A table's links are one collection: the page a row opens steps through
// the rows that have one.
test("a link column's pages are the origin, in row order, each row at its own place", () => {
  const origin = linkOrigins("Snapshots", source, column, rows, columns);
  const second = origin(rows[2]!)!;
  expect(second.title).toBe("Snapshots");
  // It keeps no items: the lens is run again when the list is wanted.
  expect(second.items).toEqual([]);
  expect(second.live).toEqual({ lens: "x/snaps", params: { n: 3 }, column: "subject" });
  expect(origin(rows[1]!)).toBeUndefined();
});

test("one page makes no collection", () => {
  expect(linkOrigins("Snapshots", source, column, [rows[0]!], columns)(rows[0]!)).toBeUndefined();
});

// The live list is the lens run now: its link column's pages, in row order.
test("a lens run resolves to its link column's pages", () => {
  const run = { lens: { columns: [{ key: "subject", link: { kind: "page", from: "uri" } }] }, result: { columns, rows } };
  expect(resolveLiveOrigin(run as never, "subject").map((i) => [i.ref.id, i.label])).toEqual([
    ["oxplow:oxplow/snapshot?id=3", "third"],
    ["oxplow:oxplow/snapshot?id=2", "second"],
  ]);
  expect(resolveLiveOrigin(run as never, "gone")).toEqual([]);
});
