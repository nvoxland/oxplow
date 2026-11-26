import { describe, expect, test } from "bun:test";

import type { PageAction } from "../tauri-bridge/generated/bindings.js";
import { chainedDrops, dragActionFor, dropBeside, dropInGroup, dropZone, keyAction, marksAfterClick, movePlaces, NOWHERE, shownMarks, withOrder } from "./rowInteract.js";

// Marks, drops and keys are rules over rows, tested without a page.

const keys = ["a", "b", "c", "d"];

test("Cmd/Ctrl+click toggles a row, Shift+click ranges from the anchor, a plain click clears", () => {
  let s = marksAfterClick([], keys, 1, null, { toggle: true, range: false });
  expect(s).toEqual({ marks: ["b"], anchor: 1 });
  s = marksAfterClick(s.marks, keys, 3, s.anchor, { toggle: false, range: true });
  expect(s).toEqual({ marks: ["b", "c", "d"], anchor: 1 });
  s = marksAfterClick(s.marks, keys, 2, s.anchor, { toggle: true, range: false });
  expect(s.marks).toEqual(["b", "d"]);
  expect(marksAfterClick(s.marks, keys, 0, s.anchor, { toggle: false, range: false })).toEqual({ marks: [], anchor: 0 });
  // A range with no anchor yet is a plain click.
  expect(marksAfterClick([], keys, 2, null, { toggle: false, range: true }).marks).toEqual([]);
});

test("marks keep drawing order and drop rows no longer shown", () => {
  expect(shownMarks(["d", "x", "a"], keys)).toEqual(["a", "d"]);
});

test("a drop's zone is the row's half, or its middle third when it takes onto", () => {
  expect(dropZone(10, 0, 30, false)).toBe("before");
  expect(dropZone(20, 0, 30, false)).toBe("after");
  expect(dropZone(15, 0, 30, true)).toBe("onto");
  expect(dropZone(5, 0, 30, true)).toBe("before");
});

const r = (uri: string) => ({ uri });

test("a drop beside a row binds it and its neighbour on the other side, if any", () => {
  const rows = [r("a"), r("b"), r("c")];
  expect(dropBeside(rows, 1, "before", "todo")).toEqual({ ...NOWHERE, before: r("b"), after: r("a"), group: "todo" });
  expect(dropBeside(rows, 2, "after", null)).toEqual({ ...NOWHERE, after: r("c") });
  expect(dropBeside(rows, 0, "before", null)).toEqual({ ...NOWHERE, before: r("a") });
  expect(dropBeside(rows, 1, "onto", null)).toEqual({ ...NOWHERE, onto: r("b") });
  expect(dropInGroup(rows, "done")).toEqual({ ...NOWHERE, after: r("c"), group: "done" });
  expect(dropInGroup([], "done")).toEqual({ ...NOWHERE, group: "done" });
});

const action = (over: Partial<PageAction>): PageAction =>
  ({ id: "x", label: "X", command: "a.b.c", input: {}, row: true, group: null, form: false, fixed: [], choices: {}, then: "open", when: null, primary: false, drag: false, bulk: false, key: null, ...over }) as PageAction;

test("a drop runs the first drag action whose places are all there", () => {
  const move = action({ id: "move", drag: true, input: { uri: "{{row.uri}}", to: "{{group.value}}" } });
  const parent = action({ id: "parent", drag: true, input: { uri: "{{row.uri}}", parent: "{{onto.uri}}" } });
  const reorder = action({ id: "reorder", drag: true, input: { uri: "{{row.uri}}", before: "{{before.uri}}" } });
  const off = action({ id: "off", drag: true, when: "open", input: {} });
  const all = [off, move, parent, reorder];
  expect(dragActionFor(all, r("a"), { ...NOWHERE, group: "done" })?.id).toBe("move");
  expect(dragActionFor(all, r("a"), { ...NOWHERE, onto: r("b") })?.id).toBe("parent");
  expect(dragActionFor(all, r("a"), { ...NOWHERE, before: r("b") })?.id).toBe("reorder");
  expect(dragActionFor([action({ id: "plain" })], r("a"), NOWHERE)).toBeNull();
});

test("a drop's order is every row with the dragged ones moved to where they landed", () => {
  const rows = [r("a"), r("b"), r("c"), r("d")];
  const ids = (at: { order: { uri: string }[] | null }) => at.order?.map((o) => o.uri);
  expect(ids(withOrder({ ...NOWHERE, before: r("b"), after: r("a") }, rows, [r("d")]))).toEqual(["a", "d", "b", "c"]);
  expect(ids(withOrder({ ...NOWHERE, after: r("c") }, rows, [r("a"), r("b")]))).toEqual(["c", "a", "b", "d"]);
  expect(ids(withOrder(NOWHERE, rows, [r("b")]))).toEqual(["a", "c", "d", "b"]);
  // A drop onto a row moves nothing.
  expect(ids(withOrder({ ...NOWHERE, onto: r("c") }, rows, [r("a")]))).toEqual(["a", "b", "c", "d"]);
});

test("an action that binds the order runs only on a drop that has one", () => {
  const order = action({ id: "order", drag: true, input: { order: "{{order.uri}}" } });
  expect(dragActionFor([order], r("a"), { ...NOWHERE, before: r("b") })).toBeNull();
  expect(dragActionFor([order], r("a"), { ...NOWHERE, before: r("b"), order: [r("a"), r("b")] })?.id).toBe("order");
});

test("rows dropped together land each after the one before it", () => {
  const at = { ...NOWHERE, before: r("z"), after: r("y") };
  expect(chainedDrops(at, [r("a"), r("b"), r("c")])).toEqual([at, { ...at, after: r("a") }, { ...at, after: r("b") }]);
  const onto = { ...NOWHERE, onto: r("p") };
  expect(chainedDrops(onto, [r("a"), r("b")])).toEqual([onto, onto]);
});

test("a key runs the row action that declares it", () => {
  const f = action({ id: "f", key: "f" });
  expect(keyAction([action({ id: "n" }), f], "f")?.id).toBe("f");
  expect(keyAction([f], "g")).toBeNull();
});

test("a key never runs a drag action: it has no drop", () => {
  expect(keyAction([action({ id: "r", key: "r", drag: true })], "r")).toBeNull();
});

// A finger cannot drag on iPad: a row's menu offers the drops a drag would
// make, as places — a step up or down in its group, into the row above
// (where a drop onto a row nests), or the end of another group.
describe("movePlaces", () => {
  const a = { id: "a" };
  const b = { id: "b" };
  const c = { id: "c" };
  const x = { id: "x" };
  test("the middle row moves up, down and into the row above", () => {
    const places = movePlaces([a, b, c], 1, "todo", [], true);
    expect(places.map((p) => p.id)).toEqual(["up", "down", "into"]);
    expect(places[0]!.place).toMatchObject({ before: a, after: null, group: "todo" });
    expect(places[1]!.place).toMatchObject({ before: null, after: c, group: "todo" });
    expect(places[2]!.place).toMatchObject({ onto: a });
  });
  test("an end row has no step past its end; no onto, no into", () => {
    expect(movePlaces([a, b], 0, null, [], false).map((p) => p.id)).toEqual(["down"]);
    expect(movePlaces([a, b], 1, null, [], false).map((p) => p.id)).toEqual(["up"]);
  });
  test("another group's end is a place; its own group is not", () => {
    const places = movePlaces([a], 0, "todo", [
      { key: "g1", label: "To do", value: "todo", rows: [a] },
      { key: "g2", label: "Done", value: "done", rows: [x] },
    ], false);
    expect(places.map((p) => [p.id, p.label])).toEqual([["group:g2", "Done"]]);
    expect(places[0]!.place).toMatchObject({ after: x, group: "done" });
  });
});
