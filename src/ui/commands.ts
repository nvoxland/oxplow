import type { MenuGroup as SharedMenuGroup, MenuItem } from "./menu.js";

export type CommandId =
  | "file.save"
  | "file.quickOpen"
  | "edit.find"
  | "view.files-sidebar"
  | "view.stream-sidebar"
  | "view.working"
  | "view.talking"
  | "view.editor";

export type MenuId = "file" | "edit" | "view";
export type MainViewId = "working" | "talking" | "editor";
export type SidebarViewId = "files" | "stream";

export interface MenuCommand extends MenuItem {
  id: CommandId;
}

export interface MenuGroup extends SharedMenuGroup {
  id: MenuId;
  label: string;
  items: MenuCommand[];
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
  showStreamSidebar(): void;
  showWorkingPane(): void;
  showTalkingPane(): void;
  showEditorPane(): void;
}

export function buildMenuGroups(state: CommandState, handlers: CommandHandlers): MenuGroup[] {
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
          run: handlers.save,
        },
        {
          id: "file.quickOpen",
          label: "Quick Open…",
          shortcut: "Ctrl/Cmd+P",
          enabled: state.hasStream,
          run: handlers.quickOpen,
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
          run: handlers.find,
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
          run: handlers.showFilesSidebar,
        },
        {
          id: "view.stream-sidebar",
          label: "Stream Sidebar",
          enabled: state.hasStream,
          checked: state.sidebarTab === "stream",
          run: handlers.showStreamSidebar,
        },
        {
          id: "view.working",
          label: "Working CC",
          enabled: state.hasStream,
          checked: state.activeTab === "working",
          run: handlers.showWorkingPane,
        },
        {
          id: "view.talking",
          label: "Talking CC",
          enabled: state.hasStream,
          checked: state.activeTab === "talking",
          run: handlers.showTalkingPane,
        },
        {
          id: "view.editor",
          label: "Editor",
          enabled: state.hasStream,
          checked: state.activeTab === "editor",
          run: handlers.showEditorPane,
        },
      ],
    },
  ];
}

export function findCommandById(groups: MenuGroup[], id: CommandId): MenuCommand | undefined {
  for (const group of groups) {
    const command = group.items.find((item) => item.id === id);
    if (command) return command;
  }
  return undefined;
}
