import { describe, expect, test } from "bun:test";

import { kindForTabId, pageKindIconComponent, pageKindLabel } from "./pageKinds.js";

describe("kindForTabId", () => {
  test("scheme-prefixed ids return the prefix", () => {
    expect(kindForTabId("file:src/foo.ts")).toBe("file");
    expect(kindForTabId("wiki:url-schemes")).toBe("wiki");
    expect(kindForTabId("work_item:oxplow:tsk42")).toBe("work_item");
    expect(kindForTabId("lens:acme/x?stream_id=2")).toBe("lens");
    expect(kindForTabId("metric:oxplow.todos")).toBe("metric");
    expect(kindForTabId("dir:src/components")).toBe("dir");
    expect(kindForTabId("commit:abcdef0")).toBe("commit");
    expect(kindForTabId("finding:fnd-1")).toBe("finding");
  });

  test("page routes return the page name, with or without params", () => {
    expect(kindForTabId("page:agent")).toBe("agent");
    expect(kindForTabId("page:tasks")).toBe("tasks");
    expect(kindForTabId("page:done-work")).toBe("done-work");
    expect(kindForTabId("page:dashboard?variant=visits")).toBe("dashboard");
    expect(kindForTabId("page:external-url?url=https://example.com")).toBe("external-url");
    expect(kindForTabId("page:diff?path=a/b.ts&left=disk&right=ref:x")).toBe("diff");
  });

  test("text that is not a ref returns itself rather than null", () => {
    expect(kindForTabId("totally-new-page")).toBe("totally-new-page");
  });
});

describe("pageKindIconComponent", () => {
  test("returns an icon for every supported scheme kind", () => {
    const supported = [
      "file",
      "directory",
      "wiki",
      "work_item",
      "commit",
      "diff",
      "duplicate-block",
      "dashboard",
      "alerts",
      "stream-settings",
      "thread-settings",
      "settings",
      "external-url",
      "uncommitted-changes",
      "tasks",
      "done-work",
      "backlog",
      "wiki-index",
      "files",
      "local-history",
      "git-history",
      "git-dashboard",
      "hook-events",
      "new-stream",
      "new-task",
      "closed-threads",
      "diff-view",
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
    expect(pageKindLabel("commit")).toBe("commit");
    expect(pageKindLabel("work_item")).toBe("work item");
    expect(pageKindLabel("metric")).toBe("metric");
    expect(pageKindLabel("wiki")).toBe("wiki page");
    expect(pageKindLabel("done-work")).toBe("done work");
    expect(pageKindLabel("local-history")).toBe("local history");
    expect(pageKindLabel("uncommitted-changes")).toBe("uncommitted");
    expect(pageKindLabel("new-task")).toBe("new item");
    expect(pageKindLabel("closed-threads")).toBe("threads");
  });

  test("passes plain kinds through unchanged", () => {
    expect(pageKindLabel("file")).toBe("file");
    expect(pageKindLabel("finding")).toBe("finding");
    expect(pageKindLabel("diff")).toBe("diff");
  });

  test("unknown kinds round-trip", () => {
    expect(pageKindLabel("custom-thing")).toBe("custom-thing");
  });
});
