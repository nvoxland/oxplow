/// The menu bar — File and Edit, the OS's on macOS and the in-window
/// `Menubar` elsewhere — built from the commands the bus offers with a
/// `ui.menu` place (`commandOffers`), in order, the project commands
/// (the shell's) among them. Beside them only the native Edit roles (Undo
/// … Select All): the OS's responder chain does those, never our
/// dispatch — WKWebView only delivers ⌘C/⌘V with them in the menu.
import type { MenuGroup as SharedMenuGroup, MenuItem } from "./menu.js";
import type { Bookmark } from "./tabs/bookmarks.js";
import type { CommandEntry } from "./components/quickOpenResults.js";
import type {
  MenuGroupSnapshot as NativeMenuGroupSnapshot,
  MenuItemSnapshot as NativeMenuItemSnapshot,
} from "./tauri-bridge/generated/bindings.js";

export type MenuId = "file" | "edit";

export interface MenuGroup extends SharedMenuGroup {
  id: MenuId;
  label: string;
  items: MenuItem[];
}

/** A menu entry and its place. */
interface Placed {
  group: string;
  order: number;
  item: MenuItem;
}

const NATIVE_EDIT: MenuItem[] = [
  { id: "native.undo", label: "Undo", enabled: true },
  { id: "native.redo", label: "Redo", enabled: true },
  { id: "native.separator.1", label: "", separator: true, enabled: true },
  { id: "native.cut", label: "Cut", enabled: true },
  { id: "native.copy", label: "Copy", enabled: true },
  { id: "native.paste", label: "Paste", enabled: true },
  { id: "native.selectAll", label: "Select All", enabled: true },
  { id: "native.separator.2", label: "", separator: true, enabled: true },
];

function entry(o: CommandEntry): MenuItem {
  return { id: o.id, label: o.label, shortcut: o.shortcut, enabled: o.enabled ?? true, run: o.run };
}

/** File and Edit: `offers` placed by `ui.menu`, after the native roles
 *  (Edit). */
export function buildMenuBar(offers: CommandEntry[]): MenuGroup[] {
  const placed = (bar: MenuId): Placed[] =>
    offers
      .filter((o) => o.menu?.bar === bar)
      .map((o) => ({ group: o.menu!.group ?? "", order: o.menu!.order, item: entry(o) }));
  // `lead` then `items` in their groups (by name), ordered within each, a
  // separator between. The shell draws only `native.separator.<digits>`
  // as one, so each is numbered on from the separators already in `lead`.
  const menu = (lead: MenuItem[], items: Placed[]): MenuItem[] => {
    items.sort((a, b) => a.group.localeCompare(b.group) || a.order - b.order);
    const out = [...lead];
    let separators = lead.filter((i) => i.separator).length;
    items.forEach((p, i) => {
      if (i > 0 && p.group !== items[i - 1].group) {
        separators += 1;
        out.push({ id: `native.separator.${separators}`, label: "", separator: true, enabled: true });
      }
      out.push(p.item);
    });
    return out;
  };
  return [
    { id: "file", label: "File", items: menu([], placed("file")) },
    { id: "edit", label: "Edit", items: menu(NATIVE_EDIT, placed("edit")) },
  ];
}

/** The menu item `id` of `groups`. */
export function menuItemById(groups: MenuGroup[], id: string): MenuItem | undefined {
  for (const group of groups) {
    const item = group.items.find((i) => i.id === id);
    if (item) return item;
  }
  return undefined;
}

/// Menu-command id prefix for a dynamic "Open Recent ▸ <project>" entry.
/// The native `menu:command` dispatch matches this prefix and opens the
/// trailing path in a new window.
export const OPEN_RECENT_PREFIX = "project.openRecent:";

/// Menu-command id prefix for a "Bookmarks ▸ <page>" entry; the rest is
/// the starred page's id (its URI), which the dispatch opens.
export const BOOKMARK_PREFIX = "bookmarks.open:";

/// The page id a Bookmarks menu id opens, or null for any other id.
export function bookmarkIdFromMenuId(menuId: string): string | null {
  return menuId.startsWith(BOOKMARK_PREFIX) ? menuId.slice(BOOKMARK_PREFIX.length) : null;
}

/// The Bookmarks items: the starred pages by title, case-insensitive; one
/// disabled hint when there are none.
export function bookmarksMenuItems(bookmarks: Bookmark[]): NativeMenuItemSnapshot[] {
  if (bookmarks.length === 0) {
    return [{ id: "bookmarks.none", label: "No bookmarks yet — star a page to add it", shortcut: null, enabled: false, checked: null }];
  }
  return bookmarks
    .map((b) => ({ id: `${BOOKMARK_PREFIX}${b.ref.id}`, label: b.label || b.ref.id, shortcut: null, enabled: true, checked: null }))
    .sort((a, b) => a.label.localeCompare(b.label, undefined, { sensitivity: "base" }) || a.id.localeCompare(b.id));
}

/// The File item that puts the `oxplow` command on the PATH (the shell's).
export const INSTALL_CLI_ID = "shell.install_cli";

/// The File item Open Recent follows.
const OPEN_RECENT_AFTER = "oxplow.project.open_in_new_window";

/// The native-menu snapshot: `groups` plus a File ▸ Open Recent ▸
/// <project> submenu built from the recents list (the in-window Menubar
/// shows `groups` as they are), then a Bookmarks menu of the starred pages.
export function buildNativeMenuSnapshots(
  groups: MenuGroup[],
  recents: { path: string; title: string; exists: boolean }[],
  bookmarks: Bookmark[] = [],
): NativeMenuGroupSnapshot[] {
  const native = groups.map((group) => {
    const items = group.items.map(nativeItem);
    if (group.id === "file") {
      const openRecent: NativeMenuItemSnapshot = {
        id: "project.openRecent",
        label: "Open Recent",
        shortcut: null,
        enabled: recents.length > 0,
        checked: null,
        submenu: recents.map((r) => ({
          id: `${OPEN_RECENT_PREFIX}${r.path}`,
          label: r.title,
          shortcut: null,
          enabled: r.exists,
          checked: null,
        })),
      };
      const afterIdx = items.findIndex((i) => i.id === OPEN_RECENT_AFTER);
      items.splice(afterIdx >= 0 ? afterIdx + 1 : items.length, 0, openRecent);
      items.push({
        id: INSTALL_CLI_ID,
        label: "Install `oxplow` command line…",
        shortcut: null,
        enabled: true,
        checked: null,
      });
    }
    return { id: group.id, label: group.label, items };
  });
  return [...native, { id: "bookmarks", label: "Bookmarks", items: bookmarksMenuItems(bookmarks) }];
}

/// A menu item as the shell's `MenuItemSnapshot`: what it draws, no more
/// (a separator is told by its `native.separator.*` id).
function nativeItem(item: MenuItem): NativeMenuItemSnapshot {
  return {
    id: item.id,
    label: item.label,
    shortcut: item.shortcut ?? null,
    enabled: item.enabled,
    checked: item.checked ?? null,
  };
}
