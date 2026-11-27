import { describe, expect, test } from "bun:test";

import { commandUri, keyedUri, knowledgeUri } from "../uri/uri.js";
import type { SearchHit } from "../api.js";
import type { PageDirectoryEntry } from "./RailHud/sections.js";
import type { PageCategory } from "./RailHud/sections.js";
import {
  buildLauncherTree,
  buildQuickOpenResults,
  buildRecentEntries,
  searchableCommands,
  type CommandEntry,
  nextSectionIndex,
  snippetParts,
  type LauncherNavRow,
  type LauncherSection,
} from "./quickOpenResults.js";

function hit(kind: string, refId: string): SearchHit {
  return {
    resource: `oxplow/${kind}`,
    uri: kind === "knowledge" ? knowledgeUri("oxwiki", refId) : keyedUri(`oxplow/${kind}`, refId),
    stream_id: null,
    title: refId,
    snippet: "…",
    tier: 5,
    score: -1,
  };
}

function page(id: string, label: string, category: PageCategory = "Work"): PageDirectoryEntry {
  return { id, label, category, ref: { id, kind: id, payload: null } as PageDirectoryEntry["ref"] };
}

function command(id: string, label: string, enabled = true): CommandEntry {
  return { id, group: "Git", label, enabled, run: () => {}, searchKey: `git ${label}`.toLowerCase() };
}

describe("searchableCommands", () => {
  test("keeps only the commands that can run now", () => {
    const out = searchableCommands([command("oxplow.vcs.commit", "Commit Changes…"), command("oxplow.vcs.pull", "Pull Changes", false)]);
    expect(out.map((c) => c.id)).toEqual(["oxplow.vcs.commit"]);
  });
});

describe("buildQuickOpenResults", () => {
  const pages = [page("oxplow:oxplow/page?page=git", "Git"), page("oxplow:oxplow/page?page=files", "Files")];
  const commands = [command("git.commit", "Commit Changes…"), command("git.pull", "Pull", false)];
  const commandHit = (id: string): SearchHit => ({ ...hit("command", id), uri: commandUri(id) });
  const pageHit = (uri: string): SearchHit => ({ ...hit("page", uri), uri });

  // A search hit answering before the pages load would be the first row,
  // and Enter would take it in place of the page the query names.
  test("until the pages have loaded, a query has no results to take", () => {
    const out = buildQuickOpenResults({ query: "Files", pages: null, commands, siteHits: [pageHit("oxplow:oxplow/page?page=files")] });
    expect(out).toEqual([]);
  });

  test("empty query is launcher mode — pages only, in given order", () => {
    const out = buildQuickOpenResults({ query: "", pages, commands, siteHits: [] });
    expect(out.map((r) => (r.kind === "page" ? r.entry.label : ""))).toEqual(["Git", "Files"]);
  });

  // The backend ranks everything on one scale: the launcher keeps its
  // order, drawing each hit as its kind's row.
  test("keeps the backend's order, each hit drawn as its kind's row", () => {
    const out = buildQuickOpenResults({
      query: "git",
      pages,
      commands,
      siteHits: [hit("file", "src/git.rs"), pageHit("oxplow:oxplow/page?page=git"), commandHit("git.commit"), hit("knowledge", "git-notes")],
    });
    expect(out.map((r) => r.kind)).toEqual(["file", "page", "command", "hit"]);
    expect(out[0]).toMatchObject({ kind: "file", path: "src/git.rs" });
    expect(out[1]).toMatchObject({ kind: "page", entry: { label: "Git" } });
    expect(out[2]).toMatchObject({ kind: "command", entry: { id: "git.commit" } });
  });

  test("a command that can't run here is left out", () => {
    const out = buildQuickOpenResults({
      query: "pull",
      pages,
      commands,
      siteHits: [commandHit("git.pull"), commandHit("not.offered"), hit("task", "tsk1")],
    });
    expect(out.map((r) => r.kind)).toEqual(["hit"]);
  });

  test("a page the launcher doesn't list is still a hit", () => {
    const out = buildQuickOpenResults({ query: "x", pages, commands, siteHits: [pageHit("oxplow:oxplow/page?page=hidden")] });
    expect(out.map((r) => r.kind)).toEqual(["hit"]);
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
  const cmd = command("g.c", "C");
  const R = {
    cat: (c: string): LauncherNavRow => ({ kind: "category", category: c as PageCategory, expanded: false }),
    page: (id: string): LauncherNavRow => ({ kind: "page", entry: page(id, id) }),
    command: (): LauncherNavRow => ({ kind: "command", entry: cmd }),
    file: (p: string): LauncherNavRow => ({ kind: "file", path: p, snippet: "" }),
    hit: (id: string): LauncherNavRow => ({ kind: "hit", hit: hit("knowledge", id) }),
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
      { refKind: "file", refId: "oxplow:oxplow/file?path=src/main.rs", label: "main.rs" },
      { refKind: "oxplow/knowledge", refId: "oxplow:oxplow/knowledge?id=architecture&store=oxwiki", label: "" },
    ]);
    expect(entries[0]!.id).toBe("recent:oxplow:oxplow/file?path=src/main.rs");
    expect(entries[0]!.label).toBe("main.rs");
    expect(entries[0]!.ref.id).toBe("oxplow:oxplow/file?path=src/main.rs");
    // Empty stored label falls back to the ref id.
    expect(entries[1]!.label).toBe("oxplow:oxplow/knowledge?id=architecture&store=oxwiki");
    expect(entries[1]!.ref.id).toBe("oxplow:oxplow/knowledge?id=architecture&store=oxwiki");
  });
});

// A content hit's snippet marks its matches with « and » (the index's
// `snippet()`): they are drawn as matches, never shown, and a markdown
// heading's `#` isn't text.
test("a snippet's marks are its matches and its markdown heading marks are dropped", () => {
  expect(snippetParts("# «QA» «note» Body text plain")).toEqual([
    { text: "QA", hit: true },
    { text: " ", hit: false },
    { text: "note", hit: true },
    { text: " Body text plain", hit: false },
  ]);
  expect(snippetParts("…before ## Heading «word» after")).toEqual([
    { text: "…before Heading ", hit: false },
    { text: "word", hit: true },
    { text: " after", hit: false },
  ]);
  expect(snippetParts("plain")).toEqual([{ text: "plain", hit: false }]);
  expect(snippetParts("issue #42 is «open»")).toEqual([
    { text: "issue #42 is ", hit: false },
    { text: "open", hit: true },
  ]);
});
