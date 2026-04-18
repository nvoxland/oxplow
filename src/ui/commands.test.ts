import { describe, expect, test } from "bun:test";
import { buildMenuGroupSnapshots, buildMenuGroups, findCommandById } from "./commands.js";

describe("buildMenuGroups", () => {
  test("disables save and find when no file is open", () => {
    const groups = buildMenuGroups(
      {
        hasStream: true,
        hasSelectedFile: false,
        canSave: false,
        activeTab: "agent",
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
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "view.editor")?.checked).toBe(true);
    expect(findCommandById(groups, "view.agent")?.checked).toBe(false);
  });

  test("disables stream-scoped commands without an active stream", () => {
    const groups = buildMenuGroups(
      {
        hasStream: false,
        hasSelectedFile: false,
        canSave: false,
        activeTab: "agent",
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "file.quickOpen")?.enabled).toBe(false);
    expect(findCommandById(groups, "view.agent")?.enabled).toBe(false);
    expect(findCommandById(groups, "view.editor")?.enabled).toBe(false);
  });
});

describe("buildMenuGroupSnapshots", () => {
  test("preserves enabled and checked state without handlers", () => {
    const groups = buildMenuGroupSnapshots({
      hasStream: true,
      hasSelectedFile: true,
      canSave: true,
      activeTab: "editor",
    });

    const viewGroup = groups.find((group) => group.id === "view");
    expect(viewGroup?.items.find((item) => item.id === "view.editor")?.checked).toBe(true);
    expect(groups.find((group) => group.id === "file")?.items.find((item) => item.id === "file.save")?.enabled).toBe(true);
  });
});

function noopHandlers() {
  return {
    save() {},
    quickOpen() {},
    find() {},
    showAgentPane() {},
    showEditorPane() {},
  };
}
