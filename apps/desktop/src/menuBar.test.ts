import { expect, test } from "bun:test";

import { buildMenuBar, buildNativeMenuSnapshots, favoriteIdFromMenuId, FAVORITE_PREFIX, favoritesMenuItems, INSTALL_CLI_ID, menuItemById, OPEN_RECENT_PREFIX } from "./menuBar.js";
import type { CommandEntry } from "./components/quickOpenResults.js";
import type { MenuItemSnapshot } from "./tauri-bridge/generated/bindings.js";

const offer = (id: string, menu: CommandEntry["menu"], enabled = true): CommandEntry => ({
  id,
  group: "G",
  label: id,
  searchKey: id,
  shortcut: "Ctrl/Cmd+S",
  menu,
  enabled,
  run: () => {},
});

test("the menu bar is the offers placed by `ui.menu`, the native Edit roles first", () => {
  const bar = buildMenuBar(
    [
      offer("oxplow.window.quick_open", { bar: "file", order: 50 }),
      offer("oxplow.project.create", { bar: "file", order: 10 }),
      offer("oxplow.editor.save", { bar: "file", order: 40 }, false),
      offer("oxplow.window.find", { bar: "edit", order: 100 }),
      offer("oxplow.vcs.pull", null),
    ],
  );
  expect(bar.map((g) => [g.id, g.items.map((i) => i.id)])).toEqual([
    ["file", ["oxplow.project.create", "oxplow.editor.save", "oxplow.window.quick_open"]],
    [
      "edit",
      [
        "native.undo",
        "native.redo",
        "native.separator.1",
        "native.cut",
        "native.copy",
        "native.paste",
        "native.selectAll",
        "native.separator.2",
        "oxplow.window.find",
      ],
    ],
  ]);
  expect(menuItemById(bar, "oxplow.editor.save")?.enabled).toBe(false);
  expect(menuItemById(bar, "oxplow.editor.save")?.shortcut).toBe("Ctrl/Cmd+S");
});

test("a menu's items sit in their groups, a separator between groups, ordered within each", () => {
  const bar = buildMenuBar([
    offer("oxplow.window.quick_open", { bar: "file", group: "3_go", order: 10 }),
    offer("oxplow.project.open", { bar: "file", group: "1_project", order: 20 }),
    offer("oxplow.project.create", { bar: "file", group: "1_project", order: 10 }),
    offer("oxplow.editor.save", { bar: "file", group: "2_save", order: 10 }),
  ]);
  expect(bar[0].items.map((i) => (i.separator ? "—" : i.id))).toEqual([
    "oxplow.project.create",
    "oxplow.project.open",
    "—",
    "oxplow.editor.save",
    "—",
    "oxplow.window.quick_open",
  ]);
  expect(bar[0].items.filter((i) => i.separator).every((i) => i.id.startsWith("native.separator."))).toBe(true);
});

// The shell draws `native.separator` or `native.separator.<digits>` as a
// separator and skips any other `native.*` id (`is_separator_role`), so a
// group's separator is numbered — unique within its menu, Edit's native
// ones included.
test("a group separator is one the shell draws: numbered, unique in its menu", () => {
  const bar = buildMenuBar([
    offer("oxplow.project.create", { bar: "file", group: "1_project", order: 10 }),
    offer("oxplow.editor.save", { bar: "file", group: "2_save", order: 10 }),
    offer("oxplow.window.quick_open", { bar: "file", group: "3_go", order: 10 }),
    offer("oxplow.window.find", { bar: "edit", group: "1_find", order: 10 }),
    offer("oxplow.window.replace", { bar: "edit", group: "2_replace", order: 10 }),
  ]);
  for (const group of bar) {
    const ids = group.items.filter((i) => i.separator).map((i) => i.id);
    expect(ids.every((id) => /^native\.separator\.\d+$/.test(id))).toBe(true);
    expect(new Set(ids).size).toBe(ids.length);
  }
});

// tsk963: the native menu is sent as the generated `MenuGroupSnapshot` the
// shell deserializes — every item names its shortcut and check state (null
// when it has none), and carries nothing the shell doesn't read.
test("the native menu is the shell's own snapshot shape, with Open Recent", () => {
  const files = [
    offer("oxplow.project.create", { bar: "file", order: 10 }),
    offer("oxplow.project.open_in_new_window", { bar: "file", order: 30 }),
    offer("oxplow.editor.save", { bar: "file", order: 40 }),
  ];
  const groups = buildNativeMenuSnapshots(buildMenuBar(files), [{ path: "/p/a", title: "A", exists: true }]);
  const keys = new Set<string>();
  const walk = (items: MenuItemSnapshot[]) => {
    for (const item of items) {
      Object.keys(item).forEach((k) => keys.add(k));
      expect(item.shortcut === null || typeof item.shortcut === "string").toBe(true);
      if (item.submenu) walk(item.submenu);
    }
  };
  for (const g of groups) walk(g.items);
  expect(keys.has("separator")).toBe(false);
  expect(keys.has("run")).toBe(false);
  const file = groups.find((g) => g.id === "file")!;
  expect(file.items.map((i) => i.id)).toEqual([
    "oxplow.project.create",
    "oxplow.project.open_in_new_window",
    "project.openRecent",
    "oxplow.editor.save",
    INSTALL_CLI_ID,
  ]);
  expect(file.items[2].submenu).toEqual([
    { id: `${OPEN_RECENT_PREFIX}/p/a`, label: "A", shortcut: null, enabled: true, checked: null },
  ]);
});

test("File ▸ Install `oxplow` command line… is in the native menu", () => {
  const file = buildNativeMenuSnapshots(buildMenuBar([]), []).find((g) => g.id === "file")!;
  const item = file.items.find((i) => i.id === INSTALL_CLI_ID);
  expect(item?.label).toBe("Install `oxplow` command line…");
  expect(item?.enabled).toBe(true);
});

const star = (id: string, label: string | null) => ({ ref: { id }, label, scope: "project" as const });

test("the Favorites menu lists the starred pages alphabetically, ignoring case", () => {
  const groups = buildNativeMenuSnapshots(buildMenuBar([]), [], [
    star("work-next", "work"),
    star("board", "Board"),
    star("alerts", "alerts"),
  ]);
  const fav = groups.find((g) => g.id === "favorites")!;
  expect(groups[groups.length - 1].id).toBe("favorites");
  expect(fav.label).toBe("Favorites");
  expect(fav.items.map((i) => i.label)).toEqual(["alerts", "Board", "work"]);
  expect(fav.items.map((i) => i.id)).toEqual([
    `${FAVORITE_PREFIX}alerts`,
    `${FAVORITE_PREFIX}board`,
    `${FAVORITE_PREFIX}work-next`,
  ]);
  expect(fav.items.every((i) => i.enabled && i.shortcut === null)).toBe(true);
});

test("a star with no label shows its page id", () => {
  const items = favoritesMenuItems([star("git-dashboard", null)]);
  expect(items[0].label).toBe("git-dashboard");
});

test("with nothing starred, Favorites is one disabled hint", () => {
  const items = favoritesMenuItems([]);
  expect(items).toEqual([
    { id: "favorites.none", label: "No favorites yet — star a page to add it", shortcut: null, enabled: false, checked: null },
  ]);
});

test("a favorite's menu id gives back the page it opens", () => {
  expect(favoriteIdFromMenuId(`${FAVORITE_PREFIX}a:b?x=1`)).toBe("a:b?x=1");
  expect(favoriteIdFromMenuId("oxplow.editor.save")).toBeNull();
});
