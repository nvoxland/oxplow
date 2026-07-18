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
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "file.save")?.enabled).toBe(false);
    expect(findCommandById(groups, "edit.find")?.enabled).toBe(false);
    expect(findCommandById(groups, "file.quickOpen")?.enabled).toBe(true);
  });

  test("exposes the tab-IA navigation View items", () => {
    const groups = buildMenuGroups(
      {
        hasStream: true,
        hasSelectedFile: true,
        canSave: true,
        hasThread: true,
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "view.files")?.enabled).toBe(true);
    expect(findCommandById(groups, "view.uncommitted")?.enabled).toBe(true);
    expect(findCommandById(groups, "view.comments")?.enabled).toBe(true);
    expect(findCommandById(groups, "view.wiki")?.enabled).toBe(true);
    expect(findCommandById(groups, "history.open")?.enabled).toBe(true);
    // Agent was removed from View; the agent tab is the pinned center tab.
    expect(findCommandById(groups, "view.agent" as never)).toBeUndefined();
  });

  test("disables stream-scoped commands without an active stream", () => {
    const groups = buildMenuGroups(
      {
        hasStream: false,
        hasSelectedFile: false,
        canSave: false,
        hasThread: false,
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "file.quickOpen")?.enabled).toBe(false);
    expect(findCommandById(groups, "view.files")?.enabled).toBe(false);
    expect(findCommandById(groups, "view.uncommitted")?.enabled).toBe(false);
    expect(findCommandById(groups, "view.comments")?.enabled).toBe(false);
  });

  test("exposes new-thread, new-stream, history commands", () => {
    const groups = buildMenuGroups(
      {
        hasStream: true,
        hasSelectedFile: false,
        canSave: false,
        hasThread: true,
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "stream.new")?.enabled).toBe(true);
    expect(findCommandById(groups, "thread.new")?.enabled).toBe(true);
    expect(findCommandById(groups, "history.open")?.enabled).toBe(true);
  });

  test("disables thread.new/history/dashboards without a stream", () => {
    const groups = buildMenuGroups(
      {
        hasStream: false,
        hasSelectedFile: false,
        canSave: false,
        hasThread: false,
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "stream.new")?.enabled).toBe(true);
    expect(findCommandById(groups, "thread.new")?.enabled).toBe(false);
    expect(findCommandById(groups, "history.open")?.enabled).toBe(false);
    expect(findCommandById(groups, "tasks.dashboard")?.enabled).toBe(false);
    expect(findCommandById(groups, "git.dashboard")?.enabled).toBe(false);
  });

  test("Git mutation commands enabled only when git is available", () => {
    const withGit = buildMenuGroups(
      {
        hasStream: true,
        hasSelectedFile: false,
        canSave: false,
        hasThread: false,
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
        canCommit: false,
      },
      noopHandlers(),
    );

    for (const id of ["git.commit", "git.pull", "git.push"] as const) {
      expect(findCommandById(withGit, id)?.enabled).toBe(true);
      expect(findCommandById(withoutGit, id)?.enabled).toBe(false);
    }
  });
});

describe("buildMenuGroupSnapshots", () => {
  test("View menu drops Agent and the relocated dashboards", () => {
    const groups = buildMenuGroupSnapshots({
      hasStream: true,
      hasSelectedFile: true,
      canSave: true,
      hasThread: true,
    });

    const viewGroup = groups.find((group) => group.id === "view");
    expect(viewGroup?.items.map((item) => item.id)).toEqual([
      "view.files",
      "view.uncommitted",
      "view.comments",
      "view.wiki",
      "history.open",
    ]);
    expect(groups.find((group) => group.id === "file")?.items.find((item) => item.id === "file.save")?.enabled).toBe(true);
  });

  test("Git menu leads with the Dashboard, then the working-tree ops", () => {
    const groups = buildMenuGroupSnapshots({
      hasStream: true,
      hasSelectedFile: false,
      canSave: false,
      hasThread: false,
      canCommit: true,
    });

    const gitGroup = groups.find((group) => group.id === "git");
    expect(gitGroup?.label).toBe("Git");
    expect(gitGroup?.items.map((item) => item.id)).toEqual([
      "git.dashboard",
      "git.commit",
      "git.pull",
      "git.push",
    ]);
  });

  test("Work menu is renamed to Tasks and leads with its Dashboard", () => {
    const groups = buildMenuGroupSnapshots({
      hasStream: true,
      hasSelectedFile: false,
      canSave: false,
      hasThread: true,
    });

    const tasksGroup = groups.find((group) => group.id === "plan");
    expect(tasksGroup?.label).toBe("Tasks");
    expect(tasksGroup?.items[0]?.id).toBe("tasks.dashboard");
    expect(tasksGroup?.items.map((item) => item.id)).toEqual([
      "tasks.dashboard",
      "plan.newTask",
      "dashboard.new",
      "thread.new",
      "stream.new",
    ]);
  });
});

function noopHandlers() {
  return {
    save() {},
    quickOpen() {},
    find() {},
    showFiles() {},
    showUncommitted() {},
    showComments() {},
    showGit() {},
    showTasks() {},
    showWiki() {},
    newTask() {},
    newStream() {},
    newDashboard() {},
    newThread() {},
    openHistory() {},
    commitFiles() {},
    pullChanges() {},
    pushChanges() {},
    openProject() {},
    openProjectNewWindow() {},
  };
}
