import type { MenuGroup as SharedMenuGroup, MenuItem } from "./menu.js";

export type CommandId =
  | "file.save"
  | "file.quickOpen"
  | "edit.find"
  | "view.files"
  | "view.uncommitted"
  | "view.comments"
  | "view.wiki"
  | "history.open"
  | "git.dashboard"
  | "git.commit"
  | "git.pull"
  | "git.push"
  | "tasks.dashboard"
  | "plan.newTask"
  | "stream.new"
  | "thread.new"
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

// `plan` is the historical id of the Tasks menu group (label "Tasks");
// the id is internal-only and kept stable so `plan.newTask` and its
// keybinding don't churn.
export type MenuId = "file" | "edit" | "view" | "git" | "plan";

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

export interface MenuGroup extends SharedMenuGroup {
  id: MenuId;
  label: string;
  items: MenuCommand[];
}

export interface MenuGroupSnapshot {
  id: MenuId;
  label: string;
  items: MenuCommandSnapshot[];
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
  showFiles(): void;
  showUncommitted(): void;
  showComments(): void;
  showGit(): void;
  showTasks(): void;
  showWiki(): void;
  newTask(): void;
  newStream(): void;
  newThread(): void;
  openHistory(): void;
  commitFiles(): void;
  pullChanges(): void;
  pushChanges(): void;
  openProject(): void;
  openProjectNewWindow(): void;
}

export function buildMenuGroupSnapshots(state: CommandState): MenuGroupSnapshot[] {
  return [
    {
      id: "file",
      label: "File",
      items: [
        { id: "project.open", label: "Open Project…", enabled: true },
        { id: "project.openNewWindow", label: "Open Project in New Window…", enabled: true },
        { id: "file.save", label: "Save", shortcut: "Ctrl/Cmd+S", enabled: state.canSave },
        { id: "file.quickOpen", label: "Quick Open…", shortcut: "Ctrl/Cmd+P", enabled: state.hasStream },
      ],
    },
    {
      id: "edit",
      label: "Edit",
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
      id: "view",
      label: "View",
      items: [
        // Tab-IA navigation: each item opens the matching page in the
        // active thread's tab set. Agent is no longer here (the agent
        // tab is the pinned center tab); the Git and Tasks dashboards
        // moved to the Git and Tasks menus respectively.
        { id: "view.files", label: "Files", enabled: state.hasStream },
        { id: "view.uncommitted", label: "Uncommitted Changes", enabled: state.hasStream },
        { id: "view.comments", label: "Comments Dashboard", enabled: state.hasStream },
        { id: "view.wiki", label: "Wiki", enabled: state.hasStream },
        { id: "history.open", label: "History", enabled: state.hasStream },
      ],
    },
    {
      id: "git",
      label: "Git",
      items: [
        // Dashboard navigates (gated on a stream); commit/pull/push are
        // mutations gated on git actually being available (`canCommit`).
        { id: "git.dashboard", label: "Dashboard", enabled: state.hasStream },
        { id: "git.commit", label: "Commit Changes…", enabled: !!state.canCommit },
        { id: "git.pull", label: "Pull Changes", enabled: !!state.canCommit },
        { id: "git.push", label: "Push Changes", enabled: !!state.canCommit },
      ],
    },
    {
      // Group id stays "plan" (see MenuId) though the label is "Tasks".
      id: "plan",
      label: "Tasks",
      items: [
        { id: "tasks.dashboard", label: "Dashboard", enabled: state.hasStream },
        { id: "plan.newTask", label: "New Task…", shortcut: "Ctrl/Cmd+Shift+N", enabled: state.hasThread },
        { id: "thread.new", label: "New Thread…", enabled: state.hasStream },
        { id: "stream.new", label: "New Stream…", enabled: true },
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
    "view.files": handlers.showFiles,
    "view.uncommitted": handlers.showUncommitted,
    "view.comments": handlers.showComments,
    "view.wiki": handlers.showWiki,
    "history.open": handlers.openHistory,
    "git.dashboard": handlers.showGit,
    "git.commit": handlers.commitFiles,
    "git.pull": handlers.pullChanges,
    "git.push": handlers.pushChanges,
    "tasks.dashboard": handlers.showTasks,
    "plan.newTask": handlers.newTask,
    "stream.new": handlers.newStream,
    "thread.new": handlers.newThread,
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

/// Native-menu snapshot item that may carry a nested submenu and a
/// free-form id (the Open Recent children use `project.openRecent:<path>`
/// ids that aren't part of the static `CommandId` union). Mirrors the
/// Rust `MenuItemSnapshot` shape (with `submenu`).
export interface NativeMenuItemSnapshot {
  id: string;
  label: string;
  shortcut?: string;
  enabled: boolean;
  checked?: boolean;
  submenu?: NativeMenuItemSnapshot[];
}

export interface NativeMenuGroupSnapshot {
  id: string;
  label: string;
  items: NativeMenuItemSnapshot[];
}

/// Menu-command id prefix for a dynamic "Open Recent ▸ <project>" entry.
/// The native `menu:command` dispatch matches this prefix and opens the
/// trailing path in a new window.
export const OPEN_RECENT_PREFIX = "project.openRecent:";

/// The native-menu snapshot: the static groups plus a dynamic
/// File ▸ Open Recent ▸ <project> submenu built from the recents list.
/// Only the native menu carries this — the in-window Menubar uses the
/// plain `buildMenuGroups`.
export function buildNativeMenuSnapshots(
  state: CommandState,
  recents: { path: string; title: string; exists: boolean }[],
): NativeMenuGroupSnapshot[] {
  return buildMenuGroupSnapshots(state).map((group) => {
    if (group.id !== "file") return group;
    const openRecent: NativeMenuItemSnapshot = {
      id: "project.openRecent",
      label: "Open Recent",
      enabled: recents.length > 0,
      submenu: recents.map((r) => ({
        id: `${OPEN_RECENT_PREFIX}${r.path}`,
        label: r.title,
        enabled: r.exists,
      })),
    };
    const items: NativeMenuItemSnapshot[] = [...group.items];
    const afterIdx = items.findIndex((i) => i.id === "project.openNewWindow");
    items.splice(afterIdx >= 0 ? afterIdx + 1 : items.length, 0, openRecent);
    return { ...group, items };
  });
}

export function findCommandById(groups: MenuGroup[], id: CommandId): MenuCommand | undefined {
  for (const group of groups) {
    const command = group.items.find((item) => item.id === id);
    if (command) return command;
  }
  return undefined;
}
