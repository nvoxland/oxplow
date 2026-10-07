import { describe, expect, test } from "bun:test";
import { buildMenuGroupSnapshots, buildMenuGroups, buildNativeMenuSnapshots, findCommandById, menuBarGroups } from "./commands.js";
import { flattenCommands } from "./components/quickOpenResults.js";

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
    expect(findCommandById(groups, "thread.new")?.enabled).toBe(false);
  });

  test("exposes the new-thread command", () => {
    const groups = buildMenuGroups(
      {
        hasStream: true,
        hasSelectedFile: false,
        canSave: false,
        hasThread: true,
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "thread.new")?.enabled).toBe(true);
  });

  test("disables thread.new without a stream", () => {
    const groups = buildMenuGroups(
      {
        hasStream: false,
        hasSelectedFile: false,
        canSave: false,
        hasThread: false,
      },
      noopHandlers(),
    );

    expect(findCommandById(groups, "thread.new")?.enabled).toBe(false);
  });

  test("New Project and Open Project run separate handlers", () => {
    const calls: string[] = [];
    const groups = buildMenuGroups(
      {
        hasStream: true,
        hasSelectedFile: false,
        canSave: false,
        hasThread: false,
      },
      {
        ...noopHandlers(),
        newProject: () => calls.push("new"),
        openProject: () => calls.push("open"),
      },
    );

    findCommandById(groups, "project.new")?.run();
    findCommandById(groups, "project.open")?.run();
    expect(calls).toEqual(["new", "open"]);
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

    for (const id of ["git.commit"] as const) {
      expect(findCommandById(withGit, id)?.enabled).toBe(true);
      expect(findCommandById(withoutGit, id)?.enabled).toBe(false);
    }
  });
});

describe("buildMenuGroupSnapshots", () => {
  test("File menu offers New Project as its own command, ahead of the Open pair", () => {
    const groups = buildMenuGroupSnapshots({
      hasStream: true,
      hasSelectedFile: false,
      canSave: false,
      hasThread: false,
    });

    const fileGroup = groups.find((group) => group.id === "file");
    expect(fileGroup?.items.map((item) => item.id)).toEqual([
      "project.new",
      "project.open",
      "project.openNewWindow",
      "file.save",
      "file.quickOpen",
    ]);
    // Creating a project is always available — it doesn't need a stream,
    // and it is the *only* command that initializes a folder.
    expect(fileGroup?.items.find((item) => item.id === "project.new")).toMatchObject({
      label: "New Project…",
      enabled: true,
    });
  });

  // Building a lens is one search entry away; it needs a thread (an agent)
  // to hand the starter prompt to.
  test("New Lens with Your Agent needs a thread", () => {
    const item = (hasThread: boolean) =>
      buildMenuGroupSnapshots({ hasStream: true, hasSelectedFile: false, canSave: false, hasThread })
        .flatMap((g) => g.items)
        .find((i) => i.id === "lens.newWithAgent");
    expect(item(true)?.label).toBe("New Lens with Your Agent…");
    expect(item(true)?.enabled).toBe(true);
    expect(item(false)?.enabled).toBe(false);
  });
});

describe("the menu bar and search", () => {
  const state = { hasStream: true, hasSelectedFile: true, canSave: true, hasThread: true, canCommit: true };

  test("the menu bar is File and Edit only", () => {
    expect(menuBarGroups(buildMenuGroups(state, noopHandlers())).map((g) => g.id)).toEqual(["file", "edit"]);
    expect(buildNativeMenuSnapshots(state, []).map((g) => g.id)).toEqual(["file", "edit"]);
  });

  test("every action off the menu bar is still a search command, under its group", () => {
    const commands = flattenCommands(buildMenuGroups(state, noopHandlers()));
    const byId = new Map(commands.map((c) => [c.id, c.group]));
    expect(Object.fromEntries(
      ["git.commit", "lens.newWithAgent", "thread.new"].map((id) => [id, byId.get(id as never)]),
    )).toEqual({
      "git.commit": "Git",
      "lens.newWithAgent": "Tasks",
      "thread.new": "Tasks",
    });
  });

  test("what the command bus offers isn't an app command too (Pull, New Task: `commandOffers`)", () => {
    const groups = buildMenuGroups(state, noopHandlers());
    for (const id of ["git.pull", "git.push", "plan.newTask", "dashboard.new", "stream.new"]) {
      expect(findCommandById(groups, id as never)).toBeUndefined();
    }
  });

  test("pages aren't commands: they're search's page rows", () => {
    const groups = buildMenuGroups(state, noopHandlers());
    for (const id of ["view.files", "view.uncommitted", "view.comments", "view.wiki", "history.open", "git.dashboard", "tasks.dashboard"]) {
      expect(findCommandById(groups, id as never)).toBeUndefined();
    }
  });
});

function noopHandlers() {
  return {
    save() {},
    quickOpen() {},
    find() {},
    newLensWithAgent() {},
    newThread() {},
    commitFiles() {},
    openProject() {},
    openProjectNewWindow() {},
    newProject() {},
  };
}
