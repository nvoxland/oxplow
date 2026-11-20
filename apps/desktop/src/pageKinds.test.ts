import { describe, expect, test } from "bun:test";

import { ICONS } from "./icons.js";
import type { Extension } from "./tauri-bridge/generated/bindings.js";
import { DEFAULT_PAGE_ICON, declaredPageIcon, kindForTabId, pageKindIconComponent, pageKindIconOrDefault, pageKindLabel } from "./pageKinds.js";

describe("kindForTabId", () => {
  test("scheme-prefixed ids return the prefix", () => {
    expect(kindForTabId("oxplow:oxplow/file?path=src/foo.ts")).toBe("oxplow/file");
    expect(kindForTabId("oxplow:oxplow/knowledge?id=url-schemes&store=oxwiki")).toBe("oxplow/knowledge");
    expect(kindForTabId("oxplow:oxplow/task?id=tsk42&provider=oxtask")).toBe("oxplow/task");
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
    for (const k of ["file", "directory", "dir", "knowledge", "task", "commit", "diff", "dashboard", "metric", "symbol"]) {
      expect(pageKindIconComponent(k), k).not.toBeNull();
    }
  });

  test("a page no extension declares draws the page icon", () => {
    expect(pageKindIconComponent("page")).not.toBeNull();
  });

  test("unknown kinds return null", () => {
    expect(pageKindIconComponent("nope")).toBeNull();
    expect(pageKindIconComponent("")).toBeNull();
  });
});

// Where a page kind is drawn (a tab, Quick Open, a backlink, a link), a
// kind nothing knows — a page an extension no longer declares — draws the
// generic page icon, so its row lines up with the others.
describe("pageKindIconOrDefault", () => {
  test("a known kind draws its own icon", () => {
    expect(pageKindIconOrDefault("task")).toBe(pageKindIconComponent("task")!);
  });

  test("an unknown kind draws the generic page icon", () => {
    expect(pageKindIconOrDefault("assistant-settings")).toBe(DEFAULT_PAGE_ICON);
    expect(pageKindIconOrDefault("")).toBe(DEFAULT_PAGE_ICON);
    expect(pageKindIconComponent("page")).toBe(DEFAULT_PAGE_ICON);
  });
});

describe("pageKindLabel", () => {
  test("rewrites hyphenated kinds to space-separated phrases", () => {
    expect(pageKindLabel("commit")).toBe("commit");
    expect(pageKindLabel("task")).toBe("task");
    expect(pageKindLabel("metric")).toBe("metric");
    expect(pageKindLabel("knowledge")).toBe("knowledge page");
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
      commit: "commit",
      dashboard: "dashboard",
      task: "task",
      knowledge: "knowledge page",
    };
    for (const [kind, word] of Object.entries(words)) {
      expect(pageKindIconComponent(kind), kind).not.toBeNull();
      expect(pageKindLabel(kind), kind).toBe(word);
    }
  });
});

describe("a declared page", () => {
  const pf = (extension: string, slug: string, title: string, icon: string | null) => ({ id: `${extension.startsWith("oxplow-") ? "oxplow" : extension.replace(/-/g, "_")}/${slug}`, extension, slug, title, icon });
  const extensions = [
    {
      name: "oxplow-foundation",
      namespace: "oxplow",
      enabled: true,
      pageFiles: [pf("oxplow-foundation", "local-history", "Local History", "history"), pf("oxplow-foundation", "plain", "Plain", null)],
    },
    { name: "gh", namespace: "gh", enabled: true, pageFiles: [pf("gh", "open-prs", "Open PRs", "git-merge")] },
    { name: "off", namespace: "off", enabled: false, pageFiles: [pf("off", "x", "X", "bell")] },
  ] as unknown as Extension[];

  test("draws the icon its declaration names, by its page id", () => {
    expect(declaredPageIcon(extensions, "local-history")).toBe(ICONS.history!);
    expect(declaredPageIcon(extensions, "gh/open-prs")).toBe(ICONS["git-merge"]!);
    expect(pageKindIconComponent("gh/open-prs", extensions)).toBe(ICONS["git-merge"]!);
  });

  test("a declared page that names no icon draws the page icon, so a list of pages has one for every row", () => {
    expect(pageKindIconComponent("plain", extensions)).toBe(pageKindIconComponent("page"));
    expect(pageKindIconComponent("gone", extensions)).toBeNull();
    expect(pageKindIconComponent("off/x", extensions)).toBeNull();
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
    { name: "oxplow-foundation", namespace: "oxplow", enabled: true, pages: [], pageFiles: [{ id: "oxplow/timeline", extension: "oxplow-foundation", slug: "timeline", title: "Time Line", icon: "history" }] },
    { name: "gh", namespace: "gh", enabled: true, pages: [], pageFiles: [{ id: "gh/open-prs", extension: "gh", slug: "open-prs", title: "Open PRs", icon: "git-merge" }] },
  ] as unknown as Extension[];

  test("come from its file, by its page id", () => {
    expect(declaredPageIcon(extensions, "timeline")).toBe(ICONS.history!);
    expect(declaredPageIcon(extensions, "gh/open-prs")).toBe(ICONS["git-merge"]!);
    expect(pageKindLabel("timeline", extensions)).toBe("time line");
  });
});
