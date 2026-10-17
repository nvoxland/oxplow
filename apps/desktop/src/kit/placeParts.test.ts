import { expect, test } from "bun:test";

import { placeParts } from "./placeParts.js";

const own = [
  { id: "start", item: "S" },
  { id: "earlier", item: "E" },
];
const ids = (placed: { id: string }[]) => placed.map((p) => p.id);

// A page's own parts keep their order; what extensions mount lands where
// each says, relative to the page's stable ids.
test("mounts land at the start, the end, or beside a named part", () => {
  const placed = placeParts(own, [
    { id: "a", at: "end", item: "A" },
    { id: "b", at: "start", item: "B" },
    { id: "c", at: { after: "start" }, item: "C" },
    { id: "d", at: { before: "earlier" }, item: "D" },
    { id: "e", at: { before: "start" }, item: "E2" },
  ]);
  expect(ids(placed)).toEqual(["b", "e", "start", "c", "d", "earlier", "a"]);
});

// Two mounts at one place keep the order they were declared in.
test("mounts at one place keep their declared order", () => {
  const placed = placeParts(own, [
    { id: "x", at: { after: "start" }, item: 1 },
    { id: "y", at: { after: "start" }, item: 2 },
    { id: "z", at: "start", item: 3 },
    { id: "w", at: "start", item: 4 },
  ]);
  expect(ids(placed)).toEqual(["z", "w", "start", "x", "y", "earlier"]);
});

// A mount naming a part the page no longer has goes at the end, so a page
// that changes never loses an extension's content.
test("a mount beside a part the page doesn't have goes at the end", () => {
  const placed = placeParts(own, [{ id: "x", at: { after: "gone" }, item: 1 }]);
  expect(ids(placed)).toEqual(["start", "earlier", "x"]);
});

test("no own parts: mounts in order, every anchor at the end", () => {
  expect(ids(placeParts([], [
    { id: "x", at: { after: "nope" }, item: 1 },
    { id: "y", at: "start", item: 2 },
  ]))).toEqual(["y", "x"]);
});
