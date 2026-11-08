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
  PageRun,
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

/// Menu-command id prefix for a "Go ▸ <page>" entry; the rest is the
/// page's URI, which the dispatch opens.
export const GO_PREFIX = "go.open:";

/// The URI a Go menu id opens, or null for any other id.
export function goUriFromMenuId(menuId: string): string | null {
  return menuId.startsWith(GO_PREFIX) ? menuId.slice(GO_PREFIX.length) : null;
}

/// One line of the Go To pane: the group it is under (Bookmarks, History,
/// Most visited), what it says and the page it opens.
export interface GoRow {
  group: string;
  title: string;
  uri: string;
}

/// The Go To pane's lines, read off its body's run (the `go-to` panel's
/// `go-to-panel` part): the loader whose rows have `grp`, `title` and `uri`.
export function goRowsOf(run: PageRun | null): GoRow[] {
  if (!run) return [];
  for (const loader of Object.values(run.loaders)) {
    const cols = loader?.result?.columns ?? [];
    const [g, t, u] = ["grp", "title", "uri"].map((c) => cols.indexOf(c));
    if (g === undefined || t === undefined || u === undefined || g < 0 || t < 0 || u < 0) continue;
    return loader!.result!.rows.flatMap((r) =>
      typeof r[u] === "string" && r[u] !== "" ? [{ group: String(r[g] ?? ""), title: String(r[t] ?? r[u]), uri: String(r[u]) }] : [],
    );
  }
  return [];
}

/// The Go items: what the Go To pane shows, each group under its heading
/// (a disabled item), a separator between; one disabled hint when empty.
export function goMenuItems(rows: GoRow[]): NativeMenuItemSnapshot[] {
  if (rows.length === 0) {
    return [{ id: "go.none", label: "Nothing yet — star a page or open one", shortcut: null, enabled: false, checked: null }];
  }
  const out: NativeMenuItemSnapshot[] = [];
  let group: string | null = null;
  let separators = 0;
  for (const row of rows) {
    if (row.group !== group) {
      if (group !== null) out.push({ id: `native.separator.${900 + separators++}`, label: "", shortcut: null, enabled: true, checked: null });
      group = row.group;
      out.push({ id: `go.heading:${group}`, label: group, shortcut: null, enabled: false, checked: null });
    }
    out.push({ id: `${GO_PREFIX}${row.uri}`, label: row.title, shortcut: null, enabled: true, checked: null });
  }
  return out;
}

/// The File item that puts the `oxplow` command on the PATH (the shell's).
export const INSTALL_CLI_ID = "shell.install_cli";

/// The File item Open Recent follows.
const OPEN_RECENT_AFTER = "oxplow.project.open_in_new_window";

/// The native-menu snapshot: `groups` plus a File ▸ Open Recent ▸
/// <project> submenu built from the recents list (the in-window Menubar
/// shows `groups` as they are), then the Go menu: what the Go To pane shows.
export function buildNativeMenuSnapshots(
  groups: MenuGroup[],
  recents: { path: string; title: string; exists: boolean }[],
  go: GoRow[] = [],
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
  return [...native, { id: "go", label: "Go", items: goMenuItems(go) }];
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
