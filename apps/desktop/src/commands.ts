import type { MenuGroup as SharedMenuGroup, MenuItem } from "./menu.js";
import type {
  MenuGroupSnapshot as NativeMenuGroupSnapshot,
  MenuItemSnapshot as NativeMenuItemSnapshot,
} from "./tauri-bridge/generated/bindings.js";

export type CommandId =
  | "file.save"
  | "file.quickOpen"
  | "edit.find"
  | "git.commit"
  | "lens.newWithAgent"
  | "thread.new"
  | "project.new"
  | "project.open"
  | "project.openNewWindow"
  // Native (responder-chain) items. Activations are dispatched by the
  // OS, never by the renderer's `menu:command` listener — the ids
  // exist only so the snapshot can carry them through to the Rust
  // menu builder, which decodes the `native.<role>` prefix.
  | "native.undo"
  | "native.redo"
  | "native.cut"
  | "native.copy"
  | "native.paste"
  | "native.selectAll"
  | "native.separator.1"
  | "native.separator.2";

// `plan` is the historical id of the Tasks group (label "Tasks"); the id
// is internal-only. Most of what the Git and Tasks groups held now comes
// from the command bus (`commandOffers`); they shrink to what doesn't yet.
export type MenuId = "file" | "edit" | "git" | "plan";

export interface MenuCommand extends MenuItem {
  id: CommandId;
}

export interface MenuCommandSnapshot {
  id: CommandId;
  label: string;
  shortcut?: string;
  enabled: boolean;
  separator?: boolean;
  checked?: boolean;
}

/** A group of commands. Every command is a search command (the
 *  launcher lists them under the group's label); only the groups
 *  `inMenuBar` also show in the OS / in-window menu bar — File and Edit.
 *  Pages aren't commands: the launcher lists them as pages. */
export interface MenuGroup extends SharedMenuGroup {
  id: MenuId;
  label: string;
  inMenuBar: boolean;
  items: MenuCommand[];
}

export interface MenuGroupSnapshot {
  id: MenuId;
  label: string;
  inMenuBar: boolean;
  items: MenuCommandSnapshot[];
}

/** The groups the menu bar shows (the native menu, the in-window Menubar). */
export function menuBarGroups<G extends { inMenuBar: boolean }>(groups: G[]): G[] {
  return groups.filter((g) => g.inMenuBar);
}

export interface CommandState {
  hasStream: boolean;
  hasSelectedFile: boolean;
  canSave: boolean;
  hasThread: boolean;
  canCommit?: boolean;
}

export interface CommandHandlers {
  save(): void;
  quickOpen(): void;
  find(): void;
  /** Put a starter "build me a lens" prompt in the agent's input (never sent). */
  newLensWithAgent(): void;
  newThread(): void;
  commitFiles(): void;
  openProject(): void;
  openProjectNewWindow(): void;
  newProject(): void;
}

export function buildMenuGroupSnapshots(state: CommandState): MenuGroupSnapshot[] {
  return [
    {
      id: "file",
      label: "File",
      inMenuBar: true,
      items: [
        // Creating and opening are separate doors: New Project… is the
        // only command that initializes a folder, and the Open pair only
        // ever opens a folder that already is one.
        { id: "project.new", label: "New Project…", enabled: true },
        { id: "project.open", label: "Open Project…", enabled: true },
        { id: "project.openNewWindow", label: "Open Project in New Window…", enabled: true },
        { id: "file.save", label: "Save", shortcut: "Ctrl/Cmd+S", enabled: state.canSave },
        { id: "file.quickOpen", label: "Quick Open…", shortcut: "Ctrl/Cmd+P", enabled: state.hasStream },
      ],
    },
    {
      id: "edit",
      label: "Edit",
      inMenuBar: true,
      items: [
        // Native (responder-chain) Cut/Copy/Paste/SelectAll. Required on
        // macOS so WKWebView delivers Cmd+V/Cmd+C/etc. to the focused
        // webview — without these items in the app menu, the standard
        // shortcuts are swallowed and JS keydown never sees them. The
        // ids `native.<role>` are decoded by the Rust menu builder
        // (see `crates/oxplow-tauri-ipc/src/commands/menu.rs`).
        { id: "native.undo", label: "Undo", enabled: true },
        { id: "native.redo", label: "Redo", enabled: true },
        { id: "native.separator.1", label: "", separator: true, enabled: true },
        { id: "native.cut", label: "Cut", enabled: true },
        { id: "native.copy", label: "Copy", enabled: true },
        { id: "native.paste", label: "Paste", enabled: true },
        { id: "native.selectAll", label: "Select All", enabled: true },
        { id: "native.separator.2", label: "", separator: true, enabled: true },
        { id: "edit.find", label: "Find", shortcut: "Ctrl/Cmd+F", enabled: state.hasSelectedFile },
      ],
    },
    {
      id: "git",
      label: "Git",
      // Search only: the Git page is a page row, these its actions.
      inMenuBar: false,
      items: [
        // Mutations, gated on git actually being available (`canCommit`).
        { id: "git.commit", label: "Commit Changes…", enabled: !!state.canCommit },
      ],
    },
    {
      // Group id stays "plan" (see MenuId) though the label is "Tasks".
      id: "plan",
      label: "Tasks",
      // Search only.
      inMenuBar: false,
      items: [
        { id: "lens.newWithAgent", label: "New Lens with Your Agent…", enabled: state.hasThread },
        { id: "thread.new", label: "New Thread…", enabled: state.hasStream },
      ],
    },
  ];
}

export function buildMenuGroups(state: CommandState, handlers: CommandHandlers): MenuGroup[] {
  const noop = () => {};
  const handlersById: Record<CommandId, () => void> = {
    "file.save": handlers.save,
    "file.quickOpen": handlers.quickOpen,
    "edit.find": handlers.find,
    "git.commit": handlers.commitFiles,
    "lens.newWithAgent": handlers.newLensWithAgent,
    "thread.new": handlers.newThread,
    "project.new": handlers.newProject,
    "project.open": handlers.openProject,
    "project.openNewWindow": handlers.openProjectNewWindow,
    // Native items dispatch through the OS responder chain.
    "native.undo": noop,
    "native.redo": noop,
    "native.cut": noop,
    "native.copy": noop,
    "native.paste": noop,
    "native.selectAll": noop,
    "native.separator.1": noop,
    "native.separator.2": noop,
  };
  return buildMenuGroupSnapshots(state).map((group) => ({
    ...group,
    items: group.items.map((item) => ({ ...item, run: handlersById[item.id] })),
  }));
}

/// Menu-command id prefix for a dynamic "Open Recent ▸ <project>" entry.
/// The native `menu:command` dispatch matches this prefix and opens the
/// trailing path in a new window.
export const OPEN_RECENT_PREFIX = "project.openRecent:";

/// The native-menu snapshot: the menu-bar groups (`inMenuBar`) plus a
/// dynamic File ▸ Open Recent ▸ <project> submenu built from the recents
/// list. Only the native menu carries this — the in-window Menubar uses
/// `menuBarGroups(buildMenuGroups(…))`.
export function buildNativeMenuSnapshots(
  state: CommandState,
  recents: { path: string; title: string; exists: boolean }[],
): NativeMenuGroupSnapshot[] {
  return menuBarGroups(buildMenuGroupSnapshots(state)).map((group) => {
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
      const afterIdx = items.findIndex((i) => i.id === "project.openNewWindow");
      items.splice(afterIdx >= 0 ? afterIdx + 1 : items.length, 0, openRecent);
    }
    return { id: group.id, label: group.label, items };
  });
}

/// A menu command as the shell's `MenuItemSnapshot`: what it draws, no
/// more (a separator is told by its `native.separator.*` id).
function nativeItem(item: MenuCommandSnapshot): NativeMenuItemSnapshot {
  return {
    id: item.id,
    label: item.label,
    shortcut: item.shortcut ?? null,
    enabled: item.enabled,
    checked: item.checked ?? null,
  };
}

export function findCommandById(groups: MenuGroup[], id: CommandId): MenuCommand | undefined {
  for (const group of groups) {
    const command = group.items.find((item) => item.id === id);
    if (command) return command;
  }
  return undefined;
}
