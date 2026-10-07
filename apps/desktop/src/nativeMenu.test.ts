import { expect, test } from "bun:test";

import { buildNativeMenuSnapshots, OPEN_RECENT_PREFIX } from "./commands.js";
import type { MenuItemSnapshot } from "./tauri-bridge/generated/bindings.js";

// tsk963: the native menu is sent as the generated `MenuGroupSnapshot` the
// shell deserializes — every item names its shortcut and check state
// (null when it has none), and carries nothing the shell doesn't read.

const state = { hasStream: true, hasSelectedFile: false, canSave: false, hasThread: true };

test("the native menu is the shell's own snapshot shape", () => {
  const groups = buildNativeMenuSnapshots(state, [{ path: "/p/a", title: "A", exists: true }]);
  const keys = new Set<string>();
  const walk = (items: MenuItemSnapshot[]) => {
    for (const item of items) {
      Object.keys(item).forEach((k) => keys.add(k));
      expect(item.shortcut === null || typeof item.shortcut === "string").toBe(true);
      expect(item.checked === null || typeof item.checked === "boolean").toBe(true);
      if (item.submenu) walk(item.submenu);
    }
  };
  for (const g of groups) walk(g.items);
  expect([...keys].sort()).toEqual(["checked", "enabled", "id", "label", "shortcut", "submenu"].filter((k) => keys.has(k)).sort());
  expect(keys.has("separator")).toBe(false);
  const file = groups.find((g) => g.id === "file")!;
  const recent = file.items.find((i) => i.id === "project.openRecent")!;
  expect(recent.submenu).toEqual([
    { id: `${OPEN_RECENT_PREFIX}/p/a`, label: "A", shortcut: null, enabled: true, checked: null },
  ]);
});
