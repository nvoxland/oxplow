import { describe, expect, test } from "bun:test";

import type { SearchHit, WorkspaceIndexedFile } from "../api.js";
import type { MenuGroup } from "../commands.js";
import type { PageDirectoryEntry } from "./RailHud/sections.js";
import type { PageCategory } from "./RailHud/sections.js";
import {
  buildLauncherTree,
  buildQuickOpenResults,
  buildRecentEntries,
  dedupeSiteHits,
  flattenCommands,
  nextSectionIndex,
  type LauncherNavRow,
  type LauncherSection,
} from "./quickOpenResults.js";

function hit(kind: string, refId: string): SearchHit {
  return {
    kind,
    ref_id: refId,
    stream_id: null,
    title: refId,
    snippet: "…",
    score: -1,
  };
}

function page(id: string, label: string, category: PageCategory = "Work"): PageDirectoryEntry {
  return { id, label, category, ref: { id, kind: id, payload: null } as PageDirectoryEntry["ref"] };
}

function file(path: string): WorkspaceIndexedFile {
  return { path, gitStatus: "clean" } as WorkspaceIndexedFile;
}

function group(items: { id: string; label: string; enabled: boolean; run?: () => void }[]): MenuGroup {
  return {
    id: "git",
    label: "Git",
    items: items.map((i) => ({ ...i, run: i.run ?? (() => {}) })),
  } as unknown as MenuGroup;
}

describe("dedupeSiteHits", () => {
  test("keeps body hits for pages and non-matching files", () => {
    const out = dedupeSiteHits(
      [hit("wiki", "architecture-overview"), hit("file", "src/other.rs"), hit("task", "12")],
      new Set(["src/main.rs"]),
    );
    expect(out.map((h) => h.ref_id)).toEqual(["architecture-overview", "src/other.rs", "12"]);
  });

  test("drops file hits whose path already matched by filename", () => {
    const out = dedupeSiteHits(
      [hit("file", "src/main.rs"), hit("wiki", "w")],
      new Set(["src/main.rs"]),
    );
    expect(out.map((h) => h.ref_id)).toEqual(["w"]);
  });

  test("non-file kinds are never deduped against paths", () => {
    const out = dedupeSiteHits([hit("wiki", "src/main.rs")], new Set(["src/main.rs"]));
    expect(out).toHaveLength(1);
  });
});

describe("flattenCommands", () => {
  test("keeps only enabled, runnable commands with a group/label searchKey", () => {
    const out = flattenCommands([
      group([
        { id: "git.commit", label: "Commit Changes…", enabled: true },
        { id: "git.pull", label: "Pull Changes", enabled: false },
      ]),
    ]);
    expect(out).toHaveLength(1);
    expect(out[0]).toMatchObject({ id: "git.commit", group: "Git", label: "Commit Changes…", searchKey: "git commit changes…" });
  });

  test("skips items with no run handler (native responder-chain placeholders)", () => {
    const out = flattenCommands([
      { id: "edit", label: "Edit", items: [{ id: "native.copy", label: "Copy", enabled: true, run: undefined }] } as unknown as MenuGroup,
    ]);
    expect(out).toHaveLength(0);
  });

  test("drops page-navigation commands that duplicate the pages directory", () => {
    const out = flattenCommands([
      group([
        { id: "tasks.dashboard", label: "Dashboard", enabled: true },
        { id: "git.dashboard", label: "Dashboard", enabled: true },
        { id: "view.files", label: "Files", enabled: true },
        { id: "view.uncommitted", label: "Uncommitted Changes", enabled: true },
        { id: "view.comments", label: "Comments Dashboard", enabled: true },
        { id: "view.wiki", label: "Wiki", enabled: true },
        { id: "history.open", label: "History", enabled: true },
        { id: "git.commit", label: "Commit Changes…", enabled: true },
      ]),
    ]);
    // Only the genuine action survives; the 7 page-nav rows are dropped
    // (the launcher shows their canonical "page" entry instead).
    expect(out.map((c) => c.id)).toEqual(["git.commit"]);
  });
});

describe("buildQuickOpenResults", () => {
  const pages = [page("git-dashboard", "Git"), page("files", "Files")];
  const commands = flattenCommands([group([{ id: "git.commit", label: "Commit Changes…", enabled: true }])]);
  const files = [file("src/git.rs"), file("README.md")];

  test("empty query is launcher mode — pages only, in given order", () => {
    const out = buildQuickOpenResults({ query: "", pages, commands, files, siteHits: [] });
    expect(out.results.map((r) => r.kind)).toEqual(["page", "page"]);
    expect(out.results.map((r) => (r.kind === "page" ? r.entry.id : ""))).toEqual(["git-dashboard", "files"]);
    expect(out.truncated).toBe(0);
  });

  test("query ranks pages → commands → files → body hits", () => {
    const out = buildQuickOpenResults({
      query: "git",
      pages,
      commands,
      files,
      siteHits: [hit("wiki", "git-notes")],
    });
    // "git" matches the Git page (label/id), the Git ▸ Commit command,
    // the src/git.rs file path, and the wiki body hit — in that order.
    // (The "Git" page label also equals the query, so the exact-match
    // hoist keeps it first — where it already was.)
    expect(out.results.map((r) => r.kind)).toEqual(["page", "command", "file", "hit"]);
  });

  test("multi-token query matches group+label of a command in any order", () => {
    const out = buildQuickOpenResults({ query: "commit git", pages, commands, files, siteHits: [] });
    expect(out.results.some((r) => r.kind === "command" && r.entry.id === "git.commit")).toBe(true);
  });

  test("an exact task-id match is hoisted to the very top (tsk51)", () => {
    const out = buildQuickOpenResults({
      query: "tsk30",
      pages,
      commands,
      files,
      siteHits: [hit("wiki", "w"), hit("task", "tsk30")],
    });
    expect(out.results[0]).toMatchObject({ kind: "hit", hit: { kind: "task", ref_id: "tsk30" } });
  });

  test("an exact content match floats above a fuzzy page/file match", () => {
    // "gt" fuzzy-matches the "Git" page and src/git.rs, but exactly
    // matches the wiki hit titled "gt" — the exact hit wins.
    const out = buildQuickOpenResults({
      query: "gt",
      pages,
      commands,
      files,
      siteHits: [hit("wiki", "gt")],
    });
    expect(out.results[0]).toMatchObject({ kind: "hit", hit: { ref_id: "gt" } });
    expect(out.results.some((r) => r.kind === "page")).toBe(true);
  });

  test("file hits from other streams are dropped; current-stream + global kept", () => {
    const fileHit = (refId: string, streamId: string | null): SearchHit => ({ ...hit("file", refId), stream_id: streamId });
    const out = buildQuickOpenResults({
      query: "rs",
      pages,
      commands,
      files: [],
      siteHits: [fileHit("a.rs", "str1"), fileHit("b.rs", "str2"), fileHit("c.rs", null)],
      currentStreamId: "str1",
    });
    const hitIds = out.results.flatMap((r) => (r.kind === "hit" ? [r.hit.ref_id] : []));
    expect(hitIds).toEqual(["a.rs", "c.rs"]); // b.rs (str2) dropped, global c.rs kept
  });

  test("truncated reports matches dropped by the file/hit caps", () => {
    const manyFiles = Array.from({ length: 85 }, (_, i) => file(`src/f${i}.ts`));
    const manyHits = Array.from({ length: 35 }, (_, i) => hit("wiki", `w${i}`));
    const out = buildQuickOpenResults({
      query: "ts",
      pages,
      commands,
      files: manyFiles,
      siteHits: manyHits,
    });
    // 85 files → 80 shown (5 dropped); 35 hits → 30 shown (5 dropped).
    expect(out.truncated).toBe(10);
  });
});

describe("buildLauncherTree", () => {
  const treePages = [
    page("tasks", "Tasks", "Work"),
    page("backlog", "Backlog", "Work"),
    page("files", "Files", "Code"),
    page("git", "Git", "Git"),
  ];
  const recentEntry = (id: string) => ({ id, label: id, ref: { id, kind: "file", payload: null } as PageDirectoryEntry["ref"] });

  test("collapsed by default — only category headers, in first-seen order", () => {
    const rows = buildLauncherTree([], treePages, new Set());
    expect(rows.map((r) => (r.kind === "category" ? r.category : `page:${r.entry.id}`))).toEqual([
      "Work",
      "Code",
      "Git",
    ]);
    expect(rows.every((r) => r.kind === "category" && !r.expanded)).toBe(true);
  });

  test("expanding a category reveals its pages beneath its header", () => {
    const rows = buildLauncherTree([], treePages, new Set<LauncherSection>(["Work"]));
    expect(rows.map((r) => (r.kind === "category" ? `${r.category}${r.expanded ? "▾" : "▸"}` : `· ${r.entry.id}`))).toEqual([
      "Work▾",
      "· tasks",
      "· backlog",
      "Code▸",
      "Git▸",
    ]);
  });

  test("Recent leads the tree when there are recent visits, expanded by default set", () => {
    const recent = [recentEntry("recent:a"), recentEntry("recent:b")];
    const rows = buildLauncherTree(recent, treePages, new Set<LauncherSection>(["Recent"]));
    expect(rows.map((r) => (r.kind === "category" ? `${r.category}${r.expanded ? "▾" : "▸"}` : `· ${r.entry.id}`))).toEqual([
      "Recent▾",
      "· recent:a",
      "· recent:b",
      "Work▸",
      "Code▸",
      "Git▸",
    ]);
  });

  test("Recent header still shows (collapsed) when not in the expanded set, hiding its rows", () => {
    const rows = buildLauncherTree([recentEntry("recent:a")], treePages, new Set());
    expect(rows[0]).toEqual({ kind: "category", category: "Recent", expanded: false });
    expect(rows.some((r) => r.kind === "page" && r.entry.id === "recent:a")).toBe(false);
  });

  test("no Recent header at all when there are no recent visits", () => {
    const rows = buildLauncherTree([], treePages, new Set<LauncherSection>(["Recent"]));
    expect(rows.some((r) => r.kind === "category" && r.category === "Recent")).toBe(false);
  });
});

describe("nextSectionIndex", () => {
  const cmd = flattenCommands([group([{ id: "g.c", label: "C", enabled: true }])])[0]!;
  const R = {
    cat: (c: string): LauncherNavRow => ({ kind: "category", category: c as PageCategory, expanded: false }),
    page: (id: string): LauncherNavRow => ({ kind: "page", entry: page(id, id) }),
    command: (): LauncherNavRow => ({ kind: "command", entry: cmd }),
    file: (p: string): LauncherNavRow => ({ kind: "file", file: file(p) }),
    hit: (id: string): LauncherNavRow => ({ kind: "hit", hit: hit("wiki", id) }),
  };

  test("search mode: Tab jumps to the first row of the next kind-run", () => {
    const rows = [R.page("a"), R.page("b"), R.command(), R.file("x"), R.hit("h1"), R.hit("h2")];
    expect(nextSectionIndex(rows, 0, 1)).toBe(2); // page-run → command
    expect(nextSectionIndex(rows, 2, 1)).toBe(3); // command → file
    expect(nextSectionIndex(rows, 3, 1)).toBe(4); // file → hit-run
    expect(nextSectionIndex(rows, 4, 1)).toBe(5); // no further boundary → clamp to last
    expect(nextSectionIndex(rows, 4, -1)).toBe(3); // back to the file run
  });

  test("launcher mode: Tab hops category header to category header", () => {
    const rows = [R.cat("Work"), R.page("a"), R.page("b"), R.cat("Code"), R.page("c")];
    expect(nextSectionIndex(rows, 0, 1)).toBe(3); // Work header → Code header
    expect(nextSectionIndex(rows, 3, 1)).toBe(4); // no further category → clamp to last
    expect(nextSectionIndex(rows, 3, -1)).toBe(0); // back to Work header
  });
});

describe("buildRecentEntries", () => {
  test("prefixes ids to avoid directory collisions, falls back label→refId, rebuilds the ref", () => {
    const entries = buildRecentEntries([
      { refKind: "file", refId: "file:src/main.rs", label: "main.rs" },
      { refKind: "wiki", refId: "wiki:architecture", label: "" },
    ]);
    expect(entries[0]!.id).toBe("recent:file:src/main.rs");
    expect(entries[0]!.label).toBe("main.rs");
    expect(entries[0]!.ref.kind).toBe("file");
    // Empty stored label falls back to the ref id.
    expect(entries[1]!.label).toBe("wiki:architecture");
    expect(entries[1]!.ref.kind).toBe("wiki");
  });
});
