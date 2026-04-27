import { describe, expect, test, beforeEach } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { WikiNoteStore, computeFreshness, type NoteRefSnapshot } from "./wiki-note-store.js";

function freshStore(): WikiNoteStore {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-wiki-note-"));
  return new WikiNoteStore(dir);
}

describe("WikiNoteStore", () => {
  let store: WikiNoteStore;
  beforeEach(() => {
    store = freshStore();
  });

  test("upsert creates a note when slug is new", () => {
    const n = store.upsert({
      slug: "hello",
      title: "Hello World",
      body: "# Hello World\n\nbody text",
      capturedHeadSha: "abc123",
      capturedRefs: [{ path: "src/foo.ts", blobSha: "deadbeef", mtimeMs: 1 }],
    });
    expect(n.slug).toBe("hello");
    expect(n.title).toBe("Hello World");
    expect(n.captured_head_sha).toBe("abc123");
    expect(n.captured_refs).toEqual([{ path: "src/foo.ts", blobSha: "deadbeef", mtimeMs: 1 }]);
    expect(n.id).toBeDefined();
    expect(n.created_at).toBeDefined();
  });

  test("upsert updates existing note without changing id or created_at", () => {
    const first = store.upsert({ slug: "a", title: "One", body: "v1", capturedHeadSha: null, capturedRefs: [] });
    const second = store.upsert({ slug: "a", title: "One updated", body: "v2", capturedHeadSha: "x", capturedRefs: [] });
    expect(second.id).toBe(first.id);
    expect(second.created_at).toBe(first.created_at);
    expect(second.title).toBe("One updated");
    expect(second.captured_head_sha).toBe("x");
  });

  test("getBySlug returns null for missing slug", () => {
    expect(store.getBySlug("nope")).toBeNull();
  });

  test("list orders by updated_at DESC", async () => {
    store.upsert({ slug: "a", title: "A", body: "", capturedHeadSha: null, capturedRefs: [] });
    await new Promise((r) => setTimeout(r, 5));
    store.upsert({ slug: "b", title: "B", body: "", capturedHeadSha: null, capturedRefs: [] });
    const list = store.list();
    expect(list.map((n) => n.slug)).toEqual(["b", "a"]);
  });

  test("deleteBySlug removes the row", () => {
    store.upsert({ slug: "x", title: "X", body: "", capturedHeadSha: null, capturedRefs: [] });
    store.deleteBySlug("x");
    expect(store.getBySlug("x")).toBeNull();
  });

  test("deleteBySlug is a no-op for missing slug", () => {
    expect(() => store.deleteBySlug("missing")).not.toThrow();
  });

  test("searchByTitle matches case-insensitively", () => {
    store.upsert({ slug: "auth", title: "Auth middleware deep dive", body: "", capturedHeadSha: null, capturedRefs: [] });
    store.upsert({ slug: "queue", title: "Work queue mechanics", body: "", capturedHeadSha: null, capturedRefs: [] });
    expect(store.searchByTitle("auth").map((n) => n.slug)).toEqual(["auth"]);
    expect(store.searchByTitle("QUEUE").map((n) => n.slug)).toEqual(["queue"]);
    expect(store.searchByTitle("middleware").map((n) => n.slug)).toEqual(["auth"]);
  });

  test("subscribe fires on upsert and delete", () => {
    const events: string[] = [];
    store.subscribe((e) => events.push(`${e.kind}:${e.slug ?? ""}`));
    store.upsert({ slug: "s", title: "S", body: "", capturedHeadSha: null, capturedRefs: [] });
    store.upsert({ slug: "s", title: "S2", body: "", capturedHeadSha: null, capturedRefs: [] });
    store.deleteBySlug("s");
    expect(events).toEqual(["upserted:s", "upserted:s", "deleted:s"]);
  });

  test("searchBodies matches case-insensitively and returns a snippet", () => {
    store.upsert({
      slug: "stop-hook",
      title: "Stop hook",
      body: "# Stop hook\n\nThe pipeline runs in priority order: commit, wait, audit, ready.",
      capturedHeadSha: null,
      capturedRefs: [],
    });
    store.upsert({
      slug: "queue",
      title: "Queue",
      body: "Sort_index ordering keeps work, commit, and wait points in one queue.",
      capturedHeadSha: null,
      capturedRefs: [],
    });
    const out = store.searchBodies("PIPELINE");
    expect(out.map((r) => r.slug)).toEqual(["stop-hook"]);
    expect(out[0]!.snippet.toLowerCase()).toContain("pipeline");
  });

  test("searchBodies via FTS5 ranks more relevant body matches higher", () => {
    store.upsert({
      slug: "auth",
      title: "Auth notes",
      body: "session token rotation; cache invalidation; cache only token",
      capturedHeadSha: null,
      capturedRefs: [],
    });
    store.upsert({
      slug: "cache",
      title: "Cache layer",
      body: "general cache discussion; LRU eviction; TTL; cache cache cache",
      capturedHeadSha: null,
      capturedRefs: [],
    });
    const out = store.searchBodies("cache");
    expect(out.map((r) => r.slug)).toEqual(["cache", "auth"]);
  });

  test("searchBodies tolerates punctuation in user query", () => {
    store.upsert({
      slug: "x", title: "X", body: "hello world!", capturedHeadSha: null, capturedRefs: [],
    });
    expect(() => store.searchBodies("hello!")).not.toThrow();
    expect(store.searchBodies("hello!").map((r) => r.slug)).toEqual(["x"]);
  });

  test("searchBodies finds rows after deletion + re-insert (FTS triggers)", () => {
    store.upsert({ slug: "a", title: "A", body: "uniqueterm here", capturedHeadSha: null, capturedRefs: [] });
    expect(store.searchBodies("uniqueterm").map((r) => r.slug)).toEqual(["a"]);
    store.deleteBySlug("a");
    expect(store.searchBodies("uniqueterm")).toEqual([]);
    store.upsert({ slug: "a", title: "A", body: "uniqueterm again", capturedHeadSha: null, capturedRefs: [] });
    expect(store.searchBodies("uniqueterm").map((r) => r.slug)).toEqual(["a"]);
  });

  test("searchBodies returns empty array for empty query", () => {
    store.upsert({ slug: "a", title: "A", body: "anything", capturedHeadSha: null, capturedRefs: [] });
    expect(store.searchBodies("")).toEqual([]);
    expect(store.searchBodies("   ")).toEqual([]);
  });

  test("findByRefPath returns notes whose captured_refs include the path", () => {
    store.upsert({
      slug: "a",
      title: "A",
      body: "",
      capturedHeadSha: null,
      capturedRefs: [{ path: "src/foo.ts", blobSha: "h1", mtimeMs: 1 }],
    });
    store.upsert({
      slug: "b",
      title: "B",
      body: "",
      capturedHeadSha: null,
      capturedRefs: [{ path: "src/bar.ts", blobSha: "h2", mtimeMs: 1 }],
    });
    store.upsert({
      slug: "c",
      title: "C",
      body: "",
      capturedHeadSha: null,
      capturedRefs: [
        { path: "src/foo.ts", blobSha: "h3", mtimeMs: 1 },
        { path: "src/bar.ts", blobSha: "h4", mtimeMs: 1 },
      ],
    });
    expect(store.findByRefPath("src/foo.ts").map((n) => n.slug).sort()).toEqual(["a", "c"]);
    expect(store.findByRefPath("src/missing.ts")).toEqual([]);
  });
});

describe("computeFreshness", () => {
  const note = {
    capturedHeadSha: "old-sha",
    capturedRefs: [
      { path: "src/a.ts", blobSha: "hash-a", mtimeMs: 1 },
      { path: "src/b.ts", blobSha: "hash-b", mtimeMs: 1 },
    ] as NoteRefSnapshot[],
  };

  test("fresh when HEAD unchanged and all blob hashes match", () => {
    const r = computeFreshness(note, "old-sha", (p) => (p === "src/a.ts" ? "hash-a" : "hash-b"));
    expect(r.status).toBe("fresh");
    expect(r.headAdvanced).toBe(false);
    expect(r.changedRefs).toEqual([]);
    expect(r.deletedRefs).toEqual([]);
  });

  test("stale when HEAD advanced", () => {
    const r = computeFreshness(note, "new-sha", (p) => (p === "src/a.ts" ? "hash-a" : "hash-b"));
    expect(r.status).toBe("stale");
    expect(r.headAdvanced).toBe(true);
  });

  test("stale when a referenced file's blob changed", () => {
    const r = computeFreshness(note, "old-sha", (p) => (p === "src/a.ts" ? "hash-a" : "different"));
    expect(r.status).toBe("stale");
    expect(r.changedRefs).toEqual(["src/b.ts"]);
  });

  test("very-stale when a referenced file is deleted", () => {
    const r = computeFreshness(note, "old-sha", (p) => (p === "src/a.ts" ? "hash-a" : null));
    expect(r.status).toBe("very-stale");
    expect(r.deletedRefs).toEqual(["src/b.ts"]);
  });

  test("when capturedHeadSha is null, HEAD is considered unchanged", () => {
    const noHead = { ...note, capturedHeadSha: null };
    const r = computeFreshness(noHead, "any-sha", () => "hash-a");
    // a.ts matches; b.ts doesn't (returns "hash-a" for both) → stale via ref mismatch, not HEAD
    expect(r.headAdvanced).toBe(false);
  });
});
