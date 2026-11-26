import { expect, test } from "bun:test";

import type { MenuGroup, MenuItem } from "../menu.js";
import { shellMenu } from "./shellMenu.js";

const item = (id: string, over: Partial<MenuItem> = {}): MenuItem => ({ id, label: id, enabled: true, run: () => {}, ...over });
const sep = (id: string): MenuItem => ({ id, label: "", enabled: true, separator: true });

// A shell with no menu bar puts the bar's groups in one menu: each group a
// submenu, without the OS's own edit items (the OS runs those, and a finger
// has the system's copy and paste), and without stray separators.
test("the menu bar's groups as submenus, the OS's items and stray separators left out", () => {
  const groups: MenuGroup[] = [
    { id: "file", label: "File", items: [item("save"), sep("s1"), item("close")] },
    { id: "edit", label: "Edit", items: [item("native.undo", { run: undefined }), sep("native.separator.1"), item("native.copy", { run: undefined }), sep("native.separator.2"), item("find")] },
    { id: "empty", label: "Empty", items: [item("native.paste", { run: undefined })] },
  ];
  const menu = shellMenu(groups);
  expect(menu.map((m) => [m.id, m.label])).toEqual([["shell.menu.file", "File"], ["shell.menu.edit", "Edit"]]);
  expect(menu[0]!.submenu!.map((m) => m.id)).toEqual(["save", "s1", "close"]);
  expect(menu[1]!.submenu!.map((m) => m.id)).toEqual(["find"]);
});
