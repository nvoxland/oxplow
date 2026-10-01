import { expect, test } from "bun:test";

import type { UiCommand } from "../tauri-bridge/generated/bindings.js";
import { bindRefInput, groupUiCommands, uiCommandMenuItems, uiCommandsAbout } from "./uiCommands.js";

const cmd = (over: Partial<UiCommand>): UiCommand => ({
  id: "x/0",
  extension: "x",
  group: "x",
  command: "x.do",
  label: "Do",
  about: "work_item",
  placement: ["menu", "context"],
  input: { ref: "{{ref}}" },
  ...over,
});

// P6b.C4: extensions' commands for a ref, by placement, grouped, with the
// ref bound into their input.
test("commands about a ref's kind, for one placement", () => {
  const all = [
    cmd({ id: "x/0" }),
    cmd({ id: "x/1", placement: ["context"] }),
    cmd({ id: "x/2", about: "commit" }),
  ];
  expect(uiCommandsAbout(all, "work_item:fake:W-1", "menu").map((c) => c.id)).toEqual(["x/0"]);
  expect(uiCommandsAbout(all, "work_item:fake:W-1", "context").map((c) => c.id)).toEqual(["x/0", "x/1"]);
  expect(uiCommandsAbout(all, "commit:abc1234", "menu").map((c) => c.id)).toEqual(["x/2"]);
  expect(uiCommandsAbout(all, "not a ref", "menu")).toEqual([]);
});

test("{{ref}} and {{ref.id}} bind as whole values, anywhere in the input", () => {
  expect(bindRefInput({ ref: "{{ref}}", id: " {{ ref.id }} ", to: "done", list: ["{{ref}}"], n: 1 }, "work_item:fake:W-1")).toEqual({
    ref: "work_item:fake:W-1",
    id: "fake:W-1",
    to: "done",
    list: ["work_item:fake:W-1"],
    n: 1,
  });
});

test("grouped by provider or extension, in first-seen order; menu items run bound", () => {
  const all = [cmd({ id: "a", group: "fake" }), cmd({ id: "b", group: "x" }), cmd({ id: "c", group: "fake", label: "Again" })];
  expect(groupUiCommands(all).map((g) => [g.group, g.commands.map((c) => c.id)])).toEqual([
    ["fake", ["a", "c"]],
    ["x", ["b"]],
  ]);
  const ran: Array<[string, unknown]> = [];
  const items = uiCommandMenuItems(all, "work_item:fake:W-1", (c, input) => ran.push([c.command, input]));
  expect(items[0]?.separator).toBe(true);
  expect(items.slice(1).map((i) => i.label)).toEqual(["fake", "x"]);
  void items[1]?.submenu?.[1]?.run?.();
  expect(ran).toEqual([["x.do", { ref: "work_item:fake:W-1" }]]);
  expect(uiCommandMenuItems([], "work_item:fake:W-1", () => {})).toEqual([]);
});
