import { describe, expect, test } from "bun:test";
import { buildMenuGroupSnapshots, buildMenuGroups, findCommandById } from "./commands.js";

describe("buildMenuGroups", () => {
  test("disables save and find when no file is open", () => {
    const groups = buildMenuGroups(
      {
        hasStream: true,
        hasSelectedFile: false,
        canSave: false,
        hasThread: false,
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
        hasThread: true,
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
        hasThread: false,
        activeTab: "agent",
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "file.quickOpen")?.enabled).toBe(false);
    expect(findCommandById(groups, "view.agent")?.enabled).toBe(false);
    expect(findCommandById(groups, "view.editor")?.enabled).toBe(false);
  });

  test("exposes new-thread, new-stream, history, snapshots commands", () => {
    const groups = buildMenuGroups(
      {
        hasStream: true,
        hasSelectedFile: false,
        canSave: false,
        hasThread: true,
        activeTab: "agent",
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "stream.new")?.enabled).toBe(true);
    expect(findCommandById(groups, "thread.new")?.enabled).toBe(true);
    expect(findCommandById(groups, "history.open")?.enabled).toBe(true);
    expect(findCommandById(groups, "snapshots.open")?.enabled).toBe(true);
  });

  test("disables thread.new/history/snapshots without a stream", () => {
    const groups = buildMenuGroups(
      {
        hasStream: false,
        hasSelectedFile: false,
        canSave: false,
        hasThread: false,
        activeTab: "agent",
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "stream.new")?.enabled).toBe(true);
    expect(findCommandById(groups, "thread.new")?.enabled).toBe(false);
    expect(findCommandById(groups, "history.open")?.enabled).toBe(false);
    expect(findCommandById(groups, "snapshots.open")?.enabled).toBe(false);
  });

  test("files.commit enabled only when git is available", () => {
    const withGit = buildMenuGroups(
      {
        hasStream: true,
        hasSelectedFile: false,
        canSave: false,
        hasThread: false,
        activeTab: "agent",
        canCommit: true,
      },
      noopHandlers(),
    );
    const withoutGit = buildMenuGroups(
      {
        hasStream: true,
        hasSelectedFile: false,
        canSave: false,
        hasThread: false,
        activeTab: "agent",
        canCommit: false,
      },
      noopHandlers(),
    );

    expect(findCommandById(withGit, "files.commit")?.enabled).toBe(true);
    expect(findCommandById(withoutGit, "files.commit")?.enabled).toBe(false);
  });
});

describe("buildMenuGroupSnapshots", () => {
  test("preserves enabled and checked state without handlers", () => {
    const groups = buildMenuGroupSnapshots({
      hasStream: true,
      hasSelectedFile: true,
      canSave: true,
      hasThread: true,
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
    newWorkItem() {},
    newStream() {},
    newThread() {},
    openHistory() {},
    openSnapshots() {},
    commitFiles() {},
  };
}
