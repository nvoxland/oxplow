import { describe, expect, test } from "bun:test";
import { buildMenuGroups, findCommandById } from "./commands.js";

describe("buildMenuGroups", () => {
  test("disables save and find when no file is open", () => {
    const groups = buildMenuGroups(
      {
        hasStream: true,
        hasSelectedFile: false,
        canSave: false,
        activeTab: "working",
        sidebarTab: "files",
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "file.save")?.enabled).toBe(false);
    expect(findCommandById(groups, "edit.find")?.enabled).toBe(false);
    expect(findCommandById(groups, "file.quickOpen")?.enabled).toBe(true);
  });

  test("marks current view targets as checked", () => {
    const groups = buildMenuGroups(
      {
        hasStream: true,
        hasSelectedFile: true,
        canSave: true,
        activeTab: "editor",
        sidebarTab: "stream",
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "view.stream-sidebar")?.checked).toBe(true);
    expect(findCommandById(groups, "view.files-sidebar")?.checked).toBe(false);
    expect(findCommandById(groups, "view.editor")?.checked).toBe(true);
    expect(findCommandById(groups, "view.working")?.checked).toBe(false);
  });

  test("disables stream-scoped commands without an active stream", () => {
    const groups = buildMenuGroups(
      {
        hasStream: false,
        hasSelectedFile: false,
        canSave: false,
        activeTab: "working",
        sidebarTab: "files",
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "file.quickOpen")?.enabled).toBe(false);
    expect(findCommandById(groups, "view.editor")?.enabled).toBe(false);
    expect(findCommandById(groups, "view.stream-sidebar")?.enabled).toBe(false);
  });
});

function noopHandlers() {
  return {
    save() {},
    quickOpen() {},
    find() {},
    showFilesSidebar() {},
    showStreamSidebar() {},
    showWorkingPane() {},
    showTalkingPane() {},
    showEditorPane() {},
  };
}
