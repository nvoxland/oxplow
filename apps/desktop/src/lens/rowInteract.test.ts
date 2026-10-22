import { expect, test } from "bun:test";

import type { LensAction } from "../tauri-bridge/generated/bindings.js";
import { chainedDrops, dragActionFor, dropBeside, dropInGroup, dropZone, keyAction, marksAfterClick, NOWHERE, shownMarks } from "./rowInteract.js";

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
  expect(dropBeside(rows, 1, "before", "todo")).toEqual({ before: r("b"), after: r("a"), onto: null, group: "todo" });
  expect(dropBeside(rows, 2, "after", null)).toEqual({ before: null, after: r("c"), onto: null, group: null });
  expect(dropBeside(rows, 0, "before", null)).toEqual({ before: r("a"), after: null, onto: null, group: null });
  expect(dropBeside(rows, 1, "onto", null)).toEqual({ ...NOWHERE, onto: r("b") });
  expect(dropInGroup(rows, "done")).toEqual({ ...NOWHERE, after: r("c"), group: "done" });
  expect(dropInGroup([], "done")).toEqual({ ...NOWHERE, group: "done" });
});

const action = (over: Partial<LensAction>): LensAction =>
  ({ id: "x", label: "X", command: "a.b.c", input: {}, row: true, group: null, form: false, fixed: [], choices: {}, then: "open", when: null, primary: false, drag: false, bulk: false, key: null, ...over }) as LensAction;

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
