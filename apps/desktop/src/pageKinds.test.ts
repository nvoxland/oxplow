import { describe, expect, test } from "bun:test";

import { ICONS } from "./icons.js";
import type { Extension } from "./tauri-bridge/generated/bindings.js";
import { declaredPageIcon, kindForTabId, pageKindIconComponent, pageKindLabel } from "./pageKinds.js";

describe("kindForTabId", () => {
  test("scheme-prefixed ids return the prefix", () => {
    expect(kindForTabId("oxplow:oxplow/file?path=src/foo.ts")).toBe("oxplow/file");
    expect(kindForTabId("oxplow:oxplow/wiki?slug=url-schemes")).toBe("oxplow/wiki");
    expect(kindForTabId("oxplow:oxplow/work_item?id=tsk42&provider=oxplow")).toBe("oxplow/work_item");
    expect(kindForTabId("oxplow:oxplow/lens?lens=acme/x&stream_id=2")).toBe("oxplow/lens");
    expect(kindForTabId("oxplow:oxplow/metric?metric=oxplow.todos")).toBe("oxplow/metric");
    expect(kindForTabId("oxplow:oxplow/dir?path=src/components")).toBe("oxplow/dir");
    expect(kindForTabId("oxplow:oxplow/commit?sha=abcdef0")).toBe("oxplow/commit");
    expect(kindForTabId("oxplow:oxplow/finding?id=fnd-1")).toBe("oxplow/finding");
    expect(kindForTabId("oxplow:oxplow/agent_session?id=ses3")).toBe("oxplow/agent_session");
  });

  test("a declared page's id is its kind, with or without params", () => {
    expect(kindForTabId("oxplow:oxplow/page?page=new-session")).toBe("new-session");
    expect(kindForTabId("oxplow:oxplow/page?page=tasks")).toBe("tasks");
    expect(kindForTabId("oxplow:oxplow/page?page=gh/open-prs")).toBe("gh/open-prs");
    expect(kindForTabId("oxplow:oxplow/page?left=disk&page=diff&path=a/b.ts&right=ref:x")).toBe("diff");
  });

  test("text that is not a ref returns itself rather than null", () => {
    expect(kindForTabId("totally-new-page")).toBe("totally-new-page");
  });
});

describe("pageKindIconComponent", () => {
  test("returns an icon for every entity kind", () => {
    for (const k of ["file", "directory", "dir", "wiki", "work_item", "commit", "diff", "dashboard", "metric", "lens", "symbol"]) {
      expect(pageKindIconComponent(k), k).not.toBeNull();
    }
  });

  test("a page no extension declares draws the lens icon", () => {
    expect(pageKindIconComponent("page")).not.toBeNull();
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

describe("core kinds a search hit can be", () => {
  test("each draws an icon and has a person's word", () => {
    const words: Record<string, string> = {
      thread: "thread",
      agent_session: "agent session",
      turn: "turn",
      effort: "effort",
      decision: "decision",
      commit: "commit",
      dashboard: "dashboard",
      lens: "lens",
      handoff: "handoff",
      comment: "comment",
      work_item: "work item",
      wiki: "wiki page",
    };
    for (const [kind, word] of Object.entries(words)) {
      expect(pageKindIconComponent(kind), kind).not.toBeNull();
      expect(pageKindLabel(kind), kind).toBe(word);
    }
  });
});

describe("a declared page", () => {
  const extensions = [
    {
      name: "oxplow-foundation",
      enabled: true,
      pages: [
        { id: "local-history", pageRef: "oxplow:oxplow/page?page=local-history", title: "Local History", icon: "history" },
        { id: "plain", pageRef: "oxplow:oxplow/page?page=plain", title: "Plain", icon: null },
      ],
    },
    { name: "gh", enabled: true, pages: [{ id: "open-prs", pageRef: "oxplow:oxplow/page?page=gh/open-prs", title: "Open PRs", icon: "git-merge" }] },
    { name: "off", enabled: false, pages: [{ id: "x", pageRef: "oxplow:oxplow/page?page=off/x", title: "X", icon: "bell" }] },
  ] as unknown as Extension[];

  test("draws the icon its declaration names, by its page id", () => {
    expect(declaredPageIcon(extensions, "local-history")).toBe(ICONS.history!);
    expect(declaredPageIcon(extensions, "gh/open-prs")).toBe(ICONS["git-merge"]!);
    expect(pageKindIconComponent("gh/open-prs", extensions)).toBe(ICONS["git-merge"]!);
  });

  test("has no icon when it names none, isn't declared, or its extension is off", () => {
    expect(declaredPageIcon(extensions, "plain")).toBeNull();
    expect(declaredPageIcon(extensions, "gone")).toBeNull();
    expect(declaredPageIcon(extensions, "off/x")).toBeNull();
    expect(declaredPageIcon(null, "local-history")).toBeNull();
  });

  test("is labelled with its title", () => {
    expect(pageKindLabel("local-history", extensions)).toBe("local history");
    expect(pageKindLabel("gh/open-prs", extensions)).toBe("open prs");
  });
});

describe("a page file's icon and label", () => {
  const extensions = [
    { name: "oxplow-foundation", namespace: "oxplow", enabled: true, pages: [], pageFiles: [{ id: "oxplow-foundation/timeline", extension: "oxplow-foundation", slug: "timeline", title: "Time Line", icon: "history" }] },
    { name: "gh", namespace: "gh", enabled: true, pages: [], pageFiles: [{ id: "gh/open-prs", extension: "gh", slug: "open-prs", title: "Open PRs", icon: "git-merge" }] },
  ] as unknown as Extension[];

  test("come from its file, by its page id", () => {
    expect(declaredPageIcon(extensions, "timeline")).toBe(ICONS.history!);
    expect(declaredPageIcon(extensions, "gh/open-prs")).toBe(ICONS["git-merge"]!);
    expect(pageKindLabel("timeline", extensions)).toBe("time line");
  });
});
