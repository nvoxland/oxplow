import { expect, test } from "bun:test";

import { terminalMenuItems } from "./terminalMenu.js";

// The terminal's right-click menu is oxplow's own, not the OS's: Copy and
// Add Selection to Context act on the selection (Copy greyed without
// one), Paste writes the clipboard, and with nothing selected Add to
// Context puts the terminal's own page (the session) in the basket.

const URI = "oxplow:oxplow/agent_session?id=ses1";

test("with a selection: Copy, Paste and Add Selection to Context", () => {
  const did: string[] = [];
  const items = terminalMenuItems({
    selection: "echo hi",
    uri: URI,
    copy: (t) => did.push(`copy:${t}`),
    paste: () => did.push("paste"),
    add: (what) => did.push(`add:${JSON.stringify(what)}`),
  });
  expect(items.map((i) => [i.id, i.label, i.enabled, i.icon ?? null])).toEqual([
    ["copy", "Copy", true, null],
    ["paste", "Paste", true, null],
    ["add-to-context", "Add Selection to Context", true, "at-sign"],
  ]);
  for (const i of items) void i.run?.();
  expect(did).toEqual(["copy:echo hi", "paste", `add:{"uri":"${URI}","text":"echo hi"}`]);
});

test("with no selection: Copy is greyed and Add to Context adds the terminal's page", () => {
  const did: string[] = [];
  const items = terminalMenuItems({ selection: "", uri: URI, copy: () => {}, paste: () => {}, add: (what) => did.push(JSON.stringify(what)) });
  expect(items.map((i) => [i.id, i.label, i.enabled])).toEqual([
    ["copy", "Copy", false],
    ["paste", "Paste", true],
    ["add-to-context", "Add to Context", true],
  ]);
  void items[2]!.run?.();
  expect(did).toEqual([`{"uri":"${URI}"}`]);
});
