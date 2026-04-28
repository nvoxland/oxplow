import { describe, expect, test, beforeEach } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { WikiNoteThreadUpdateStore } from "./wiki-note-thread-update-store.js";

function freshStore(): WikiNoteThreadUpdateStore {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-wiki-note-thread-update-"));
  return new WikiNoteThreadUpdateStore(dir);
}

describe("WikiNoteThreadUpdateStore", () => {
  let store: WikiNoteThreadUpdateStore;
  beforeEach(() => {
    store = freshStore();
  });

  test("recordUpdate inserts then upserts in place per (slug, thread_id)", () => {
    store.recordUpdate("hello", "b-thread-1", "2026-01-01T00:00:00.000Z");
    store.recordUpdate("hello", "b-thread-1", "2026-01-02T00:00:00.000Z");
    const rows = store.listRecentByThread("b-thread-1", 10);
    expect(rows).toHaveLength(1);
    expect(rows[0]?.updated_at).toBe("2026-01-02T00:00:00.000Z");
  });

  test("distinct (slug, thread_id) rows coexist", () => {
    store.recordUpdate("hello", "b-thread-1", "2026-01-01T00:00:00.000Z");
    store.recordUpdate("hello", "b-thread-2", "2026-01-02T00:00:00.000Z");
    expect(store.listRecentByThread("b-thread-1", 10)).toHaveLength(1);
    expect(store.listRecentByThread("b-thread-2", 10)).toHaveLength(1);
  });

  test("listRecentByThread returns rows newest-first and respects limit", () => {
    store.recordUpdate("a", "b-thread-1", "2026-01-01T00:00:00.000Z");
    store.recordUpdate("b", "b-thread-1", "2026-01-03T00:00:00.000Z");
    store.recordUpdate("c", "b-thread-1", "2026-01-02T00:00:00.000Z");
    const rows = store.listRecentByThread("b-thread-1", 2);
    expect(rows.map((r) => r.slug)).toEqual(["b", "c"]);
  });

  test("deleteBySlug clears every thread's attribution for that slug", () => {
    store.recordUpdate("hello", "b-thread-1", "2026-01-01T00:00:00.000Z");
    store.recordUpdate("hello", "b-thread-2", "2026-01-02T00:00:00.000Z");
    store.recordUpdate("other", "b-thread-1", "2026-01-03T00:00:00.000Z");
    store.deleteBySlug("hello");
    expect(store.listRecentByThread("b-thread-1", 10).map((r) => r.slug)).toEqual(["other"]);
    expect(store.listRecentByThread("b-thread-2", 10)).toEqual([]);
  });
});
