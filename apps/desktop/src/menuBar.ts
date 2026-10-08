/// The menu bar — File and Edit, the OS's on macOS and the in-window
/// `Menubar` elsewhere — built from the commands the bus offers with a
/// `ui.menu` place (`commandOffers`), in order, the project commands
/// (the shell's) among them. Beside them only the native Edit roles (Undo
/// … Select All): the OS's responder chain does those, never our
/// dispatch — WKWebView only delivers ⌘C/⌘V with them in the menu.
import type { MenuGroup as SharedMenuGroup, MenuItem } from "./menu.js";
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
    offers.filter((o) => o.menu?.bar === bar).map((o) => ({ order: o.menu!.order, item: entry(o) }));
  const sorted = (items: Placed[]) => items.sort((a, b) => a.order - b.order).map((p) => p.item);
  return [
    { id: "file", label: "File", items: sorted(placed("file")) },
    { id: "edit", label: "Edit", items: [...NATIVE_EDIT, ...sorted(placed("edit"))] },
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

/// The File item Open Recent follows.
const OPEN_RECENT_AFTER = "oxplow.project.open_in_new_window";

/// The native-menu snapshot: `groups` plus a File ▸ Open Recent ▸
/// <project> submenu built from the recents list (the in-window Menubar
/// shows `groups` as they are).
export function buildNativeMenuSnapshots(
  groups: MenuGroup[],
  recents: { path: string; title: string; exists: boolean }[],
): NativeMenuGroupSnapshot[] {
  return groups.map((group) => {
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
    }
    return { id: group.id, label: group.label, items };
  });
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
