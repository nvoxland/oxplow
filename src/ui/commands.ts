import type { MenuGroup as SharedMenuGroup, MenuItem } from "./menu.js";

export type CommandId =
  | "file.save"
  | "file.quickOpen"
  | "edit.find"
  | "view.files-sidebar"
  | "view.batches-sidebar"
  | "view.stream-sidebar"
  | "view.agent"
  | "view.plan"
  | "view.editor";

export type MenuId = "file" | "edit" | "view";
export type MainViewId = "agent" | "plan" | "editor";
export type SidebarViewId = "files" | "batches" | "stream";

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
  activeTab: MainViewId;
  sidebarTab: SidebarViewId;
}

export interface CommandHandlers {
  save(): void;
  quickOpen(): void;
  find(): void;
  showFilesSidebar(): void;
  showBatchesSidebar(): void;
  showStreamSidebar(): void;
  showAgentPane(): void;
  showPlanPane(): void;
  showEditorPane(): void;
}

export function buildMenuGroupSnapshots(state: CommandState): MenuGroupSnapshot[] {
  return [
    {
      id: "file",
      label: "File",
      items: [
        {
          id: "file.save",
          label: "Save",
          shortcut: "Ctrl/Cmd+S",
          enabled: state.canSave,
        },
        {
          id: "file.quickOpen",
          label: "Quick Open…",
          shortcut: "Ctrl/Cmd+P",
          enabled: state.hasStream,
        },
      ],
    },
    {
      id: "edit",
      label: "Edit",
      items: [
        {
          id: "edit.find",
          label: "Find",
          shortcut: "Ctrl/Cmd+F",
          enabled: state.hasSelectedFile,
        },
      ],
    },
    {
      id: "view",
      label: "View",
      items: [
        {
          id: "view.files-sidebar",
          label: "Files Sidebar",
          enabled: state.hasStream,
          checked: state.sidebarTab === "files",
        },
        {
          id: "view.batches-sidebar",
          label: "Batch Queue",
          enabled: state.hasStream,
          checked: state.sidebarTab === "batches",
        },
        {
          id: "view.stream-sidebar",
          label: "Stream Sidebar",
          enabled: state.hasStream,
          checked: state.sidebarTab === "stream",
        },
        {
          id: "view.agent",
          label: "Agent",
          enabled: state.hasStream,
          checked: state.activeTab === "agent",
        },
        {
          id: "view.plan",
          label: "Plan",
          enabled: state.hasStream,
          checked: state.activeTab === "plan",
        },
        {
          id: "view.editor",
          label: "Editor",
          enabled: state.hasStream,
          checked: state.activeTab === "editor",
        },
      ],
    },
  ];
}

export function buildMenuGroups(state: CommandState, handlers: CommandHandlers): MenuGroup[] {
  const handlersById: Record<CommandId, () => void> = {
    "file.save": handlers.save,
    "file.quickOpen": handlers.quickOpen,
    "edit.find": handlers.find,
    "view.files-sidebar": handlers.showFilesSidebar,
    "view.batches-sidebar": handlers.showBatchesSidebar,
    "view.stream-sidebar": handlers.showStreamSidebar,
    "view.agent": handlers.showAgentPane,
    "view.plan": handlers.showPlanPane,
    "view.editor": handlers.showEditorPane,
  };
  return buildMenuGroupSnapshots(state).map((group) => ({
    ...group,
    items: group.items.map((item) => ({
      ...item,
      run: handlersById[item.id],
    })),
  }));
}

export function findCommandById(groups: MenuGroup[], id: CommandId): MenuCommand | undefined {
  for (const group of groups) {
    const command = group.items.find((item) => item.id === id);
    if (command) return command;
  }
  return undefined;
}
