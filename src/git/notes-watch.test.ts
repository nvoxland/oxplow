import { describe, expect, test, beforeEach } from "bun:test";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { WikiNoteStore } from "../persistence/wiki-note-store.js";
import { syncNoteFromDisk, scanAndSyncAll } from "./notes-watch.js";

function freshProject(): string {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-notes-watch-"));
  // Initialize git so captured_head_sha can be populated (not strictly required)
  execFileSync("git", ["init", "-q", dir]);
  execFileSync("git", ["-C", dir, "config", "user.email", "t@t.com"]);
  execFileSync("git", ["-C", dir, "config", "user.name", "T"]);
  mkdirSync(join(dir, ".oxplow", "notes"), { recursive: true });
  return dir;
}

describe("syncNoteFromDisk", () => {
  let projectDir: string;
  let store: WikiNoteStore;
  beforeEach(() => {
    projectDir = freshProject();
    store = new WikiNoteStore(projectDir);
  });

  test("creates a note from a file's body", () => {
    const file = join(projectDir, ".oxplow", "notes", "hello.md");
    writeFileSync(file, "# Hello\n\nSee src/foo.ts.");
    writeFileSync(join(projectDir, "src-foo.ts"), "x"); // unrelated
    syncNoteFromDisk(projectDir, store, "hello");
    const n = store.getBySlug("hello");
    expect(n).not.toBeNull();
    expect(n!.title).toBe("Hello");
    expect(n!.captured_refs.map((r) => r.path)).toEqual(["src/foo.ts"]);
    // ref was parsed but file doesn't exist → blobSha = null
    expect(n!.captured_refs[0]!.blobSha).toBeNull();
  });

  test("falls back to slug when no # heading present", () => {
    const file = join(projectDir, ".oxplow", "notes", "untitled.md");
    writeFileSync(file, "just prose");
    syncNoteFromDisk(projectDir, store, "untitled");
    expect(store.getBySlug("untitled")!.title).toBe("untitled");
  });

  test("hashes referenced files that exist on disk", () => {
    mkdirSync(join(projectDir, "src"), { recursive: true });
    writeFileSync(join(projectDir, "src", "foo.ts"), "export const x = 1");
    const file = join(projectDir, ".oxplow", "notes", "n.md");
    writeFileSync(file, "# N\n\nsrc/foo.ts is here");
    syncNoteFromDisk(projectDir, store, "n");
    const refs = store.getBySlug("n")!.captured_refs;
    expect(refs.length).toBe(1);
    expect(refs[0]!.path).toBe("src/foo.ts");
    expect(refs[0]!.blobSha).toMatch(/^[0-9a-f]{64}$/);
  });

  test("deletes the row when the file no longer exists", () => {
    const file = join(projectDir, ".oxplow", "notes", "tmp.md");
    writeFileSync(file, "# Tmp");
    syncNoteFromDisk(projectDir, store, "tmp");
    expect(store.getBySlug("tmp")).not.toBeNull();
    rmSync(file);
    syncNoteFromDisk(projectDir, store, "tmp");
    expect(store.getBySlug("tmp")).toBeNull();
  });
});

describe("scanAndSyncAll", () => {
  test("syncs every .md file and removes orphan rows", () => {
    const projectDir = freshProject();
    const store = new WikiNoteStore(projectDir);
    writeFileSync(join(projectDir, ".oxplow", "notes", "a.md"), "# A");
    writeFileSync(join(projectDir, ".oxplow", "notes", "b.md"), "# B");
    // Pre-seed a row whose file will be missing → should be deleted
    store.upsert({ slug: "orphan", title: "Orphan", body: "", capturedHeadSha: null, capturedRefs: [] });

    scanAndSyncAll(projectDir, store);

    const slugs = store.list().map((n) => n.slug).sort();
    expect(slugs).toEqual(["a", "b"]);
  });
});
