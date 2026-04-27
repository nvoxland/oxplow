import { beforeEach, describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { UsageStore } from "./usage-store.js";

function freshStore(coalesceMs = 30_000): UsageStore {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-usage-"));
  return new UsageStore(dir, undefined, coalesceMs);
}

const ISO = (ms: number) => new Date(ms).toISOString();

describe("UsageStore", () => {
  let store: UsageStore;
  beforeEach(() => {
    store = freshStore();
  });

  test("record + mostRecent returns rows newest-first", () => {
    store.record({ kind: "wiki-note", key: "a", streamId: "s1", occurredAt: ISO(1_000_000) });
    store.record({ kind: "wiki-note", key: "b", streamId: "s1", occurredAt: ISO(2_000_000) });
    const rows = store.mostRecent({ kind: "wiki-note", streamId: "s1" });
    expect(rows.map((r) => r.key)).toEqual(["b", "a"]);
    expect(rows[0]!.count).toBe(1);
  });

  test("coalesces same (kind, key, event) within window — bumps occurred_at, no new row", () => {
    store.record({ kind: "wiki-note", key: "a", streamId: "s1", occurredAt: ISO(1_000_000) });
    store.record({ kind: "wiki-note", key: "a", streamId: "s1", occurredAt: ISO(1_005_000) }); // 5s later
    const rows = store.mostRecent({ kind: "wiki-note", streamId: "s1" });
    expect(rows).toHaveLength(1);
    expect(rows[0]!.count).toBe(1);
    expect(rows[0]!.last_at).toBe(ISO(1_005_000));
  });

  test("does NOT coalesce when outside window", () => {
    store.record({ kind: "wiki-note", key: "a", streamId: "s1", occurredAt: ISO(1_000_000) });
    store.record({ kind: "wiki-note", key: "a", streamId: "s1", occurredAt: ISO(1_060_000) }); // 60s later
    const rows = store.mostRecent({ kind: "wiki-note", streamId: "s1" });
    expect(rows[0]!.count).toBe(2);
  });

  test("mostFrequent ranks by count then last_at", () => {
    store = freshStore(0);
    store.record({ kind: "wiki-note", key: "a", streamId: "s1", occurredAt: ISO(1) });
    store.record({ kind: "wiki-note", key: "a", streamId: "s1", occurredAt: ISO(2) });
    store.record({ kind: "wiki-note", key: "a", streamId: "s1", occurredAt: ISO(3) });
    store.record({ kind: "wiki-note", key: "b", streamId: "s1", occurredAt: ISO(4) });
    const rows = store.mostFrequent({ kind: "wiki-note", streamId: "s1" });
    expect(rows.map((r) => r.key)).toEqual(["a", "b"]);
    expect(rows[0]!.count).toBe(3);
  });

  test("currentlyOpen returns keys whose latest event is open with no later close", () => {
    store = freshStore(0);
    store.record({ kind: "editor-file", key: "a.ts", event: "open", streamId: "s1", occurredAt: ISO(1) });
    store.record({ kind: "editor-file", key: "b.ts", event: "open", streamId: "s1", occurredAt: ISO(2) });
    store.record({ kind: "editor-file", key: "a.ts", event: "close", streamId: "s1", occurredAt: ISO(3) });
    expect(store.currentlyOpen({ kind: "editor-file", streamId: "s1" })).toEqual(["b.ts"]);
  });

  test("filters by streamId when provided", () => {
    store = freshStore(0);
    store.record({ kind: "wiki-note", key: "a", streamId: "s1", occurredAt: ISO(1) });
    store.record({ kind: "wiki-note", key: "b", streamId: "s2", occurredAt: ISO(2) });
    expect(store.mostRecent({ kind: "wiki-note", streamId: "s1" }).map((r) => r.key)).toEqual(["a"]);
    expect(store.mostRecent({ kind: "wiki-note", streamId: "s2" }).map((r) => r.key)).toEqual(["b"]);
    expect(store.mostRecent({ kind: "wiki-note" }).map((r) => r.key).sort()).toEqual(["a", "b"]);
  });

  test("filters by `since`", () => {
    store = freshStore(0);
    store.record({ kind: "wiki-note", key: "old", streamId: "s1", occurredAt: ISO(1_000) });
    store.record({ kind: "wiki-note", key: "new", streamId: "s1", occurredAt: ISO(2_000) });
    const rows = store.mostRecent({ kind: "wiki-note", streamId: "s1", since: ISO(1_500) });
    expect(rows.map((r) => r.key)).toEqual(["new"]);
  });

  test("subscribe fires on every record (including coalesce)", () => {
    const events: string[] = [];
    store.subscribe((c) => events.push(`${c.kind}:${c.key}:${c.streamId ?? ""}`));
    store.record({ kind: "wiki-note", key: "a", streamId: "s1", occurredAt: ISO(1_000_000) });
    store.record({ kind: "wiki-note", key: "a", streamId: "s1", occurredAt: ISO(1_005_000) }); // coalesced
    expect(events).toEqual(["wiki-note:a:s1", "wiki-note:a:s1"]);
  });

  test("filters by threadId when provided", () => {
    store = freshStore(0);
    store.record({ kind: "editor-file", key: "a.ts", streamId: "s1", threadId: "t1", occurredAt: ISO(1) });
    store.record({ kind: "editor-file", key: "b.ts", streamId: "s1", threadId: "t2", occurredAt: ISO(2) });
    expect(store.mostRecent({ kind: "editor-file", threadId: "t1" }).map((r) => r.key)).toEqual(["a.ts"]);
    expect(store.mostRecent({ kind: "editor-file", threadId: "t2" }).map((r) => r.key)).toEqual(["b.ts"]);
    expect(
      store.mostRecent({ kind: "editor-file", streamId: "s1", threadId: "t1" }).map((r) => r.key),
    ).toEqual(["a.ts"]);
  });

  test("coalesce respects threadId — same key in different threads stays distinct", () => {
    store.record({ kind: "editor-file", key: "a.ts", streamId: "s1", threadId: "t1", occurredAt: ISO(1_000_000) });
    store.record({ kind: "editor-file", key: "a.ts", streamId: "s1", threadId: "t2", occurredAt: ISO(1_000_500) });
    const allRows = store.mostRecent({ kind: "editor-file", streamId: "s1" });
    expect(allRows[0]!.count).toBe(2);
  });

  test("pruneOlderThan deletes rows below cutoff", () => {
    store = freshStore(0);
    store.record({ kind: "wiki-note", key: "old", streamId: "s1", occurredAt: ISO(1_000) });
    store.record({ kind: "wiki-note", key: "new", streamId: "s1", occurredAt: ISO(5_000) });
    store.pruneOlderThan(ISO(2_000));
    expect(store.mostRecent({ kind: "wiki-note", streamId: "s1" }).map((r) => r.key)).toEqual(["new"]);
  });
});
