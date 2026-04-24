import { describe, expect, test, beforeEach } from "bun:test";
import { mkdtempSync, mkdirSync, writeFileSync, existsSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import type { Stream } from "../persistence/stream-store.js";
import { WikiNoteStore } from "../persistence/wiki-note-store.js";
import { syncNoteFromDisk } from "../git/notes-watch.js";
import { buildWikiNoteMcpTools } from "./wiki-note-mcp-tools.js";

function freshProject(): { projectDir: string; stream: Stream } {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-wn-mcp-"));
  execFileSync("git", ["init", "-q", dir]);
  execFileSync("git", ["-C", dir, "config", "user.email", "t@t.com"]);
  execFileSync("git", ["-C", dir, "config", "user.name", "T"]);
  mkdirSync(join(dir, ".oxplow", "notes"), { recursive: true });
  const stream: Stream = {
    id: "s1",
    title: "S",
    summary: "",
    branch: "main",
    branch_ref: "refs/heads/main",
    branch_source: "local",
    worktree_path: dir,
    working_pane: "p:w",
    talking_pane: "p:t",
    working_session_id: "",
    talking_session_id: "",
    sort_index: 0,
    current_snapshot_id: null,
    custom_prompt: null,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  } as Stream;
  return { projectDir: dir, stream };
}

function findTool(tools: ReturnType<typeof buildWikiNoteMcpTools>, name: string) {
  const t = tools.find((x) => x.name === name);
  if (!t) throw new Error(`tool not found: ${name}`);
  return t;
}

describe("wiki note MCP tools", () => {
  let projectDir: string;
  let stream: Stream;
  let store: WikiNoteStore;
  let tools: ReturnType<typeof buildWikiNoteMcpTools>;
  beforeEach(() => {
    const fresh = freshProject();
    projectDir = fresh.projectDir;
    stream = fresh.stream;
    store = new WikiNoteStore(projectDir);
    tools = buildWikiNoteMcpTools({ resolveStream: () => stream, wikiNoteStore: store });
  });

  test("list_notes returns summaries with freshness=fresh for a note with no refs", async () => {
    writeFileSync(join(projectDir, ".oxplow", "notes", "intro.md"), "# Intro\n\nhi");
    syncNoteFromDisk(projectDir, store, "intro");
    const tool = findTool(tools, "oxplow__list_notes");
    const out: any = await tool.handler({});
    expect(out.notes.length).toBe(1);
    expect(out.notes[0].slug).toBe("intro");
    expect(out.notes[0].title).toBe("Intro");
    expect(out.notes[0].freshness).toBe("fresh");
    expect(out.notes[0].path.endsWith(".oxplow/notes/intro.md")).toBe(true);
  });

  test("get_note_metadata reports very-stale when a referenced file disappears", async () => {
    mkdirSync(join(projectDir, "src"), { recursive: true });
    writeFileSync(join(projectDir, "src", "foo.ts"), "x");
    writeFileSync(join(projectDir, ".oxplow", "notes", "n.md"), "# N\n\nSee src/foo.ts");
    syncNoteFromDisk(projectDir, store, "n");

    // Delete the referenced file
    execFileSync("rm", [join(projectDir, "src", "foo.ts")]);

    const tool = findTool(tools, "oxplow__get_note_metadata");
    const out: any = await tool.handler({ slug: "n" });
    expect(out.freshness).toBe("very-stale");
    expect(out.deleted_refs).toEqual(["src/foo.ts"]);
  });

  test("resync_note re-baselines freshness after the agent writes", async () => {
    // Agent writes the file directly (simulated).
    writeFileSync(join(projectDir, ".oxplow", "notes", "x.md"), "# X");

    const tool = findTool(tools, "oxplow__resync_note");
    const out: any = await tool.handler({ slug: "x" });
    expect(out.slug).toBe("x");
    expect(out.freshness).toBe("fresh");
    expect(store.getBySlug("x")).not.toBeNull();
  });

  test("delete_note removes file and row", async () => {
    writeFileSync(join(projectDir, ".oxplow", "notes", "del.md"), "# Del");
    syncNoteFromDisk(projectDir, store, "del");
    expect(store.getBySlug("del")).not.toBeNull();

    const tool = findTool(tools, "oxplow__delete_note");
    const out: any = await tool.handler({ slug: "del" });
    expect(out.deleted).toBe(true);
    expect(store.getBySlug("del")).toBeNull();
    expect(existsSync(join(projectDir, ".oxplow", "notes", "del.md"))).toBe(false);
  });

  test("slug validation rejects traversal", () => {
    const tool = findTool(tools, "oxplow__get_note_metadata");
    expect(() => tool.handler({ slug: "../etc/passwd" })).toThrow();
    expect(() => tool.handler({ slug: "sub/dir" })).toThrow();
  });

  test("search_notes filters by title substring (case-insensitive)", async () => {
    writeFileSync(join(projectDir, ".oxplow", "notes", "a.md"), "# Auth dive");
    writeFileSync(join(projectDir, ".oxplow", "notes", "b.md"), "# Queue internals");
    syncNoteFromDisk(projectDir, store, "a");
    syncNoteFromDisk(projectDir, store, "b");
    const tool = findTool(tools, "oxplow__search_notes");
    const out: any = await tool.handler({ query: "AUTH" });
    expect(out.notes.map((n: any) => n.slug)).toEqual(["a"]);
  });

  test("list_notes staleOnly filter excludes fresh notes", async () => {
    writeFileSync(join(projectDir, ".oxplow", "notes", "fresh.md"), "# Fresh");
    syncNoteFromDisk(projectDir, store, "fresh");
    const tool = findTool(tools, "oxplow__list_notes");
    const out: any = await tool.handler({ staleOnly: true });
    expect(out.notes).toEqual([]);
  });

  // Sanity: the file we wrote is actually on disk where the path points.
  test("list_notes path points to the actual file on disk", async () => {
    writeFileSync(join(projectDir, ".oxplow", "notes", "p.md"), "# P\n\nbody");
    syncNoteFromDisk(projectDir, store, "p");
    const tool = findTool(tools, "oxplow__list_notes");
    const out: any = await tool.handler({});
    const entry = out.notes.find((n: any) => n.slug === "p");
    expect(entry).toBeDefined();
    expect(readFileSync(entry.path, "utf8")).toContain("body");
  });
});
