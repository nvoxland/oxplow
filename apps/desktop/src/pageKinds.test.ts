import { describe, expect, test } from "bun:test";

import { REF_KIND_ICONS } from "./refKinds.js";
import type { Extension } from "./tauri-bridge/generated/bindings.js";
import { extPageIconComponent, kindForTabId, pageKindIconComponent, pageKindLabel } from "./pageKinds.js";

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
    expect(kindForTabId("agent_session:ses3")).toBe("agent_session");
  });

  test("page routes return the page name, with or without params", () => {
    expect(kindForTabId("page:new-session")).toBe("new-session");
    expect(kindForTabId("page:tasks")).toBe("tasks");
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
      "backlog",
      "wiki-index",
      "files",
      "local-history",
      "git-history",
      "git-dashboard",
      "new-stream",
      "new-task",
      "diff-view",
    ];
    for (const k of supported) {
      expect(pageKindIconComponent(k)).not.toBeNull();
    }
  });

  test("an agent session's tab and the session picker have icons and labels", () => {
    expect(pageKindIconComponent("agent_session")).not.toBeNull();
    expect(pageKindIconComponent("new-session")).not.toBeNull();
    expect(pageKindLabel("agent_session")).toBe("agent session");
    expect(pageKindLabel("new-session")).toBe("new session");
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
    expect(pageKindLabel("local-history")).toBe("local history");
    expect(pageKindLabel("uncommitted-changes")).toBe("uncommitted");
    expect(pageKindLabel("new-task")).toBe("new item");
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
      thread_note: "note",
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

describe("an extension page's icon", () => {
  const extensions = [
    {
      name: "oxplow-foundation",
      pages: [
        { id: "problems", icon: "triangle-alert" },
        { id: "plain", icon: null },
      ],
    },
  ] as unknown as Extension[];

  test("is the one its page declares", () => {
    expect(extPageIconComponent(extensions, "page:oxplow-foundation/problems")).toBe(REF_KIND_ICONS["triangle-alert"]!);
  });

  test("is none when the page declares none, or is not there", () => {
    expect(extPageIconComponent(extensions, "page:oxplow-foundation/plain")).toBeNull();
    expect(extPageIconComponent(extensions, "page:oxplow-foundation/gone")).toBeNull();
    expect(extPageIconComponent(null, "page:oxplow-foundation/problems")).toBeNull();
    expect(extPageIconComponent(extensions, "page:tasks")).toBeNull();
  });
});
