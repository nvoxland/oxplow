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
    const first = store.upsert({ slug: "a", title: "One", capturedHeadSha: null, capturedRefs: [] });
    const second = store.upsert({ slug: "a", title: "One updated", capturedHeadSha: "x", capturedRefs: [] });
    expect(second.id).toBe(first.id);
    expect(second.created_at).toBe(first.created_at);
    expect(second.title).toBe("One updated");
    expect(second.captured_head_sha).toBe("x");
  });

  test("getBySlug returns null for missing slug", () => {
    expect(store.getBySlug("nope")).toBeNull();
  });

  test("list orders by updated_at DESC", async () => {
    store.upsert({ slug: "a", title: "A", capturedHeadSha: null, capturedRefs: [] });
    await new Promise((r) => setTimeout(r, 5));
    store.upsert({ slug: "b", title: "B", capturedHeadSha: null, capturedRefs: [] });
    const list = store.list();
    expect(list.map((n) => n.slug)).toEqual(["b", "a"]);
  });

  test("deleteBySlug removes the row", () => {
    store.upsert({ slug: "x", title: "X", capturedHeadSha: null, capturedRefs: [] });
    store.deleteBySlug("x");
    expect(store.getBySlug("x")).toBeNull();
  });

  test("deleteBySlug is a no-op for missing slug", () => {
    expect(() => store.deleteBySlug("missing")).not.toThrow();
  });

  test("searchByTitle matches case-insensitively", () => {
    store.upsert({ slug: "auth", title: "Auth middleware deep dive", capturedHeadSha: null, capturedRefs: [] });
    store.upsert({ slug: "queue", title: "Work queue mechanics", capturedHeadSha: null, capturedRefs: [] });
    expect(store.searchByTitle("auth").map((n) => n.slug)).toEqual(["auth"]);
    expect(store.searchByTitle("QUEUE").map((n) => n.slug)).toEqual(["queue"]);
    expect(store.searchByTitle("middleware").map((n) => n.slug)).toEqual(["auth"]);
  });

  test("subscribe fires on upsert and delete", () => {
    const events: string[] = [];
    store.subscribe((e) => events.push(`${e.kind}:${e.slug ?? ""}`));
    store.upsert({ slug: "s", title: "S", capturedHeadSha: null, capturedRefs: [] });
    store.upsert({ slug: "s", title: "S2", capturedHeadSha: null, capturedRefs: [] });
    store.deleteBySlug("s");
    expect(events).toEqual(["upserted:s", "upserted:s", "deleted:s"]);
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
