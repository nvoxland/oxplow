import { expect, test } from "bun:test";

import { childrenFirst, laneLayout, parentsOf } from "./lanes.js";

// `format: lanes`: each row's part of the lanes of the graph the rows form,
// as `git log --graph` keeps them, in the order the rows are drawn.

test("a straight history is one lane, a child before its parent", () => {
  const { rows, width } = laneLayout([
    { id: "c", parents: ["b"] },
    { id: "b", parents: ["a"] },
    { id: "a", parents: [] },
  ]);
  expect(width).toBe(1);
  expect(rows.map((r) => r.column)).toEqual([0, 0, 0]);
  expect(rows.map((r) => r.fromAbove)).toEqual([false, true, true]);
  // The root has nothing below it.
  expect(rows[2]!.outgoing).toEqual([]);
});

test("a merge's second parent opens the next lane, which folds back at the common parent", () => {
  const { rows, width } = laneLayout([
    { id: "d", parents: ["c", "b"] },
    { id: "c", parents: ["a"] },
    { id: "b", parents: ["a"] },
    { id: "a", parents: [] },
  ]);
  expect(width).toBe(2);
  expect(rows.map((r) => r.column)).toEqual([0, 0, 1, 0]);
  // The merge draws an edge into lane 1 from its own row.
  expect(rows[0]!.edges).toEqual([0, 1]);
  // b's first parent is a, already waited for in lane 0: b's line folds back.
  expect(rows[2]!.edges).toEqual([0]);
});

test("a parent the rows don't hold opens no lane past its child", () => {
  const { rows } = laneLayout([
    { id: "b", parents: ["gone"] },
    { id: "a", parents: [] },
  ]);
  expect(rows[0]!.outgoing).toEqual([]);
  expect(rows[1]!.column).toBe(0);
  expect(rows[1]!.fromAbove).toBe(false);
});

test("a parents cell is a JSON array of ids; anything else is none", () => {
  expect(parentsOf('["a","b"]')).toEqual(["a", "b"]);
  expect(parentsOf("[]")).toEqual([]);
  expect(parentsOf(null)).toEqual([]);
  expect(parentsOf("not json")).toEqual([]);
  expect(parentsOf('["a",null]')).toEqual(["a"]);
});

test("rows draw children before parents, otherwise in their order", () => {
  // b's parent a was committed later than b, so the loader put it first.
  const order = childrenFirst([
    { id: "a", parents: [] },
    { id: "c", parents: ["b"] },
    { id: "b", parents: ["a"] },
    { id: "x", parents: [] },
  ]);
  expect(order).toEqual([1, 2, 0, 3]);
  // In that order, one lane: no lane waits for a parent already drawn.
  const nodes = [{ id: "a", parents: [] }, { id: "c", parents: ["b"] }, { id: "b", parents: ["a"] }, { id: "x", parents: [] }];
  expect(laneLayout(order.map((i) => nodes[i]!)).width).toBe(1);
});
