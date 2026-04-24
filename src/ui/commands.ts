import type { MenuGroup as SharedMenuGroup, MenuItem } from "./menu.js";

export type CommandId =
  | "file.save"
  | "file.quickOpen"
  | "edit.find"
  | "view.agent"
  | "view.editor"
  | "history.open"
  | "snapshots.open"
  | "plan.newWorkItem"
  | "stream.new"
  | "thread.new"
  | "files.commit";

export type MenuId = "file" | "edit" | "view" | "plan";
export type MainViewId = "agent" | "editor";

export interface MenuCommand extends MenuItem {
  id: CommandId;
}

export interface MenuCommandSnapshot {
  id: CommandId;
  label: string;
  shortcut?: string;
  enabled: boolean;
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
  activeTab: MainViewId;
  canCommit?: boolean;
}

export interface CommandHandlers {
  save(): void;
  quickOpen(): void;
  find(): void;
  showAgentPane(): void;
  showEditorPane(): void;
  newWorkItem(): void;
  newStream(): void;
  newThread(): void;
  openHistory(): void;
  openSnapshots(): void;
  commitFiles(): void;
}

export function buildMenuGroupSnapshots(state: CommandState): MenuGroupSnapshot[] {
  return [
    {
      id: "file",
      label: "File",
      items: [
        { id: "file.save", label: "Save", shortcut: "Ctrl/Cmd+S", enabled: state.canSave },
        { id: "file.quickOpen", label: "Quick Open…", shortcut: "Ctrl/Cmd+P", enabled: state.hasStream },
        { id: "files.commit", label: "Commit Changes…", enabled: !!state.canCommit },
      ],
    },
    {
      id: "edit",
      label: "Edit",
      items: [
        { id: "edit.find", label: "Find", shortcut: "Ctrl/Cmd+F", enabled: state.hasSelectedFile },
      ],
    },
    {
      id: "view",
      label: "View",
      items: [
        { id: "view.agent", label: "Agent", enabled: state.hasStream, checked: state.activeTab === "agent" },
        { id: "view.editor", label: "Editor", enabled: state.hasStream, checked: state.activeTab === "editor" },
        { id: "history.open", label: "History", enabled: state.hasStream },
        { id: "snapshots.open", label: "Snapshots", enabled: state.hasStream },
      ],
    },
    {
      id: "plan",
      label: "Work",
      items: [
        { id: "plan.newWorkItem", label: "New Task…", shortcut: "Ctrl/Cmd+Shift+N", enabled: state.hasThread },
        { id: "thread.new", label: "New Thread…", enabled: state.hasStream },
        { id: "stream.new", label: "New Stream…", enabled: true },
      ],
    },
  ];
}

export function buildMenuGroups(state: CommandState, handlers: CommandHandlers): MenuGroup[] {
  const handlersById: Record<CommandId, () => void> = {
    "file.save": handlers.save,
    "file.quickOpen": handlers.quickOpen,
    "edit.find": handlers.find,
    "view.agent": handlers.showAgentPane,
    "view.editor": handlers.showEditorPane,
    "plan.newWorkItem": handlers.newWorkItem,
    "stream.new": handlers.newStream,
    "thread.new": handlers.newThread,
    "history.open": handlers.openHistory,
    "snapshots.open": handlers.openSnapshots,
    "files.commit": handlers.commitFiles,
  };
  return buildMenuGroupSnapshots(state).map((group) => ({
    ...group,
    items: group.items.map((item) => ({ ...item, run: handlersById[item.id] })),
  }));
}

export function findCommandById(groups: MenuGroup[], id: CommandId): MenuCommand | undefined {
  for (const group of groups) {
    const command = group.items.find((item) => item.id === id);
    if (command) return command;
  }
  return undefined;
}
