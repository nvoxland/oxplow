import { describe, expect, test } from "bun:test";

import type { SearchHit, WorkspaceIndexedFile } from "../api.js";
import type { MenuGroup } from "../commands.js";
import type { PageDirectoryEntry } from "./RailHud/sections.js";
import { buildQuickOpenResults, dedupeSiteHits, flattenCommands } from "./quickOpenResults.js";

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

function page(id: string, label: string): PageDirectoryEntry {
  return { id, label, category: "Work", ref: { id, kind: id, payload: null } as PageDirectoryEntry["ref"] };
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
});

describe("buildQuickOpenResults", () => {
  const pages = [page("git-dashboard", "Git"), page("files", "Files")];
  const commands = flattenCommands([group([{ id: "git.commit", label: "Commit Changes…", enabled: true }])]);
  const files = [file("src/git.rs"), file("README.md")];

  test("empty query is launcher mode — pages only, in given order", () => {
    const out = buildQuickOpenResults({ query: "", pages, commands, files, siteHits: [] });
    expect(out.map((r) => r.kind)).toEqual(["page", "page"]);
    expect(out.map((r) => (r.kind === "page" ? r.entry.id : ""))).toEqual(["git-dashboard", "files"]);
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
    expect(out.map((r) => r.kind)).toEqual(["page", "command", "file", "hit"]);
  });

  test("multi-token query matches group+label of a command in any order", () => {
    const out = buildQuickOpenResults({ query: "commit git", pages, commands, files, siteHits: [] });
    expect(out.some((r) => r.kind === "command" && r.entry.id === "git.commit")).toBe(true);
  });
});
