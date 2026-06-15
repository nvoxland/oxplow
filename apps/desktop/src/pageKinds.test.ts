import { describe, expect, test } from "bun:test";

import { kindForTabId, pageKindIconComponent, pageKindLabel } from "./pageKinds.js";

describe("kindForTabId", () => {
  test("scheme-prefixed ids return the prefix", () => {
    expect(kindForTabId("file:src/foo.ts")).toBe("file");
    expect(kindForTabId("wiki:url-schemes")).toBe("wiki");
    expect(kindForTabId("task:42")).toBe("task");
    expect(kindForTabId("dir:src/components")).toBe("dir");
    expect(kindForTabId("git-commit:abcdef0")).toBe("git-commit");
    expect(kindForTabId("git-commit:abc:scope:value")).toBe("git-commit");
    expect(kindForTabId("dashboard:planning")).toBe("dashboard");
    expect(kindForTabId("external-url:https://example.com")).toBe("external-url");
    expect(kindForTabId("finding:fnd-1")).toBe("finding");
  });

  test("literal index ids return themselves", () => {
    expect(kindForTabId("agent")).toBe("agent");
    expect(kindForTabId("tasks")).toBe("tasks");
    expect(kindForTabId("done-work")).toBe("done-work");
    expect(kindForTabId("wiki-index")).toBe("wiki-index");
    expect(kindForTabId("files")).toBe("files");
    expect(kindForTabId("settings")).toBe("settings");
    expect(kindForTabId("uncommitted-changes")).toBe("uncommitted-changes");
  });

  test("uncommitted-changes with scope suffix still resolves to the kind", () => {
    expect(kindForTabId("uncommitted-changes:dir:src")).toBe("uncommitted-changes");
  });

  test("unknown bare ids return themselves rather than null", () => {
    expect(kindForTabId("totally-new-page")).toBe("totally-new-page");
  });
});

describe("pageKindIconComponent", () => {
  test("returns an icon for every supported scheme kind", () => {
    const supported = [
      "file",
      "directory",
      "wiki",
      "task",
      "finding",
      "git-commit",
      "diff",
      "duplicate-block",
      "dashboard",
      "op-error",
      "stream-settings",
      "thread-settings",
      "settings",
      "external-url",
      "uncommitted-changes",
      "tasks",
      "done-work",
      "backlog",
      "archived",
      "wiki-index",
      "files",
      "code-quality",
      "local-history",
      "git-history",
      "git-dashboard",
      "hook-events",
      "new-stream",
      "new-task",
      "closed-threads",
      "snapshot",
    ];
    for (const k of supported) {
      expect(pageKindIconComponent(k)).not.toBeNull();
    }
  });

  test("agent tab is intentionally iconless", () => {
    // The agent tab is always present and unambiguous; an icon
    // there would just widen the chip. Suppress.
    expect(pageKindIconComponent("agent")).toBeNull();
  });

  test("unknown kinds return null", () => {
    expect(pageKindIconComponent("nope")).toBeNull();
    expect(pageKindIconComponent("")).toBeNull();
  });
});

describe("pageKindLabel", () => {
  test("rewrites hyphenated kinds to space-separated phrases", () => {
    expect(pageKindLabel("git-commit")).toBe("commit");
    expect(pageKindLabel("wiki")).toBe("wiki page");
    expect(pageKindLabel("done-work")).toBe("done work");
    expect(pageKindLabel("local-history")).toBe("local history");
    expect(pageKindLabel("uncommitted-changes")).toBe("uncommitted");
    expect(pageKindLabel("new-task")).toBe("new task");
    expect(pageKindLabel("closed-threads")).toBe("threads");
  });

  test("passes plain kinds through unchanged", () => {
    expect(pageKindLabel("file")).toBe("file");
    expect(pageKindLabel("task")).toBe("task");
    expect(pageKindLabel("finding")).toBe("finding");
    expect(pageKindLabel("diff")).toBe("diff");
  });

  test("unknown kinds round-trip", () => {
    expect(pageKindLabel("custom-thing")).toBe("custom-thing");
  });
});
