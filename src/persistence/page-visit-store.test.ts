import { beforeEach, describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PageVisitStore } from "./page-visit-store.js";

function freshStore(): PageVisitStore {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-page-visit-"));
  return new PageVisitStore(dir);
}

const ISO = (ms: number) => new Date(ms).toISOString();

describe("PageVisitStore", () => {
  let store: PageVisitStore;
  beforeEach(() => {
    store = freshStore();
  });

  test("records and lists newest-first", () => {
    store.record({ refKind: "plan-work", refId: "plan-work", payload: null, label: "Plan work", occurredAt: ISO(1) });
    store.record({ refKind: "files", refId: "files", payload: null, label: "Files", occurredAt: ISO(2) });
    const rows = store.listRecent({ limit: 10 });
    expect(rows.map((r) => r.refId)).toEqual(["files", "plan-work"]);
    expect(rows[0]!.label).toBe("Files");
  });

  test("dedupeByRef collapses to one row per ref_id, newest visit wins", () => {
    store.record({ refKind: "plan-work", refId: "plan-work", payload: null, label: "Plan work", occurredAt: ISO(1) });
    store.record({ refKind: "files", refId: "files", payload: null, label: "Files", occurredAt: ISO(2) });
    store.record({ refKind: "plan-work", refId: "plan-work", payload: null, label: "Plan work", occurredAt: ISO(3) });
    const rows = store.listRecent({ limit: 10, dedupeByRef: true });
    expect(rows.map((r) => r.refId)).toEqual(["plan-work", "files"]);
  });

  test("limit truncates results", () => {
    for (let i = 1; i <= 5; i++) {
      store.record({ refKind: "files", refId: `id-${i}`, payload: null, label: `f${i}`, occurredAt: ISO(i) });
    }
    expect(store.listRecent({ limit: 3 })).toHaveLength(3);
  });

  test("threadId scope filters out other threads", () => {
    store.record({ refKind: "plan-work", refId: "p", payload: null, label: "p", threadId: "t1", occurredAt: ISO(1) });
    store.record({ refKind: "files", refId: "f", payload: null, label: "f", threadId: "t2", occurredAt: ISO(2) });
    expect(store.listRecent({ limit: 10, threadId: "t1" }).map((r) => r.refId)).toEqual(["p"]);
  });

  test("excludeKinds filters specified ref kinds", () => {
    store.record({ refKind: "agent", refId: "a", payload: null, label: "a", occurredAt: ISO(1) });
    store.record({ refKind: "plan-work", refId: "p", payload: null, label: "p", occurredAt: ISO(2) });
    expect(
      store.listRecent({ limit: 10, excludeKinds: ["agent"] }).map((r) => r.refId),
    ).toEqual(["p"]);
  });

  test("payload roundtrips through JSON", () => {
    store.record({
      refKind: "work-item",
      refId: "wi-1",
      payload: { itemId: "wi-1", extra: { nested: true } },
      label: "Work item 1",
    });
    const r = store.listRecent({ limit: 1 })[0]!;
    expect(r.payload).toEqual({ itemId: "wi-1", extra: { nested: true } });
  });

  test("topVisited orders by count DESC then last_t DESC", () => {
    store.record({ refKind: "files", refId: "f1", payload: null, label: "F1", occurredAt: ISO(1) });
    store.record({ refKind: "files", refId: "f1", payload: null, label: "F1", occurredAt: ISO(2) });
    store.record({ refKind: "files", refId: "f1", payload: null, label: "F1", occurredAt: ISO(3) });
    store.record({ refKind: "files", refId: "f2", payload: null, label: "F2", occurredAt: ISO(4) });
    const rows = store.topVisited({ limit: 5 });
    expect(rows.map((r) => r.refId)).toEqual(["f1", "f2"]);
    expect(rows[0]!.count).toBe(3);
  });

  test("topVisited sinceT excludes earlier rows", () => {
    store.record({ refKind: "files", refId: "f1", payload: null, label: "F1", occurredAt: ISO(1) });
    store.record({ refKind: "files", refId: "f2", payload: null, label: "F2", occurredAt: ISO(50) });
    const rows = store.topVisited({ limit: 5, sinceT: ISO(10) });
    expect(rows.map((r) => r.refId)).toEqual(["f2"]);
  });

  test("countByDay groups by date prefix", () => {
    store.record({ refKind: "files", refId: "f", payload: null, label: "F", occurredAt: "2026-04-27T10:00:00.000Z" });
    store.record({ refKind: "files", refId: "f", payload: null, label: "F", occurredAt: "2026-04-27T15:00:00.000Z" });
    store.record({ refKind: "files", refId: "f", payload: null, label: "F", occurredAt: "2026-04-28T08:00:00.000Z" });
    expect(store.countByDay({ refId: "f" })).toEqual([
      { day: "2026-04-27", count: 2 },
      { day: "2026-04-28", count: 1 },
    ]);
  });

  test("subscribe fires on record", () => {
    const events: string[] = [];
    store.subscribe((c) => events.push(c.refId));
    store.record({ refKind: "files", refId: "f1", payload: null, label: "F" });
    expect(events).toEqual(["f1"]);
  });

  test("pruneOlderThan deletes rows before cutoff", () => {
    store.record({ refKind: "files", refId: "old", payload: null, label: "O", occurredAt: ISO(1) });
    store.record({ refKind: "files", refId: "new", payload: null, label: "N", occurredAt: ISO(100) });
    store.pruneOlderThan(ISO(50));
    const rows = store.listRecent({ limit: 10 });
    expect(rows.map((r) => r.refId)).toEqual(["new"]);
  });

  test("forget removes every visit for a refKind+refId and emits change", () => {
    store.record({ refKind: "op-error", refId: "op-error:oe-1", payload: null, label: "E1", occurredAt: ISO(1) });
    store.record({ refKind: "op-error", refId: "op-error:oe-1", payload: null, label: "E1", occurredAt: ISO(2) });
    store.record({ refKind: "op-error", refId: "op-error:oe-2", payload: null, label: "E2", occurredAt: ISO(3) });
    const events: string[] = [];
    store.subscribe((c) => events.push(`${c.refKind}:${c.refId}`));
    store.forget("op-error", "op-error:oe-1");
    const rows = store.listRecent({ limit: 10 });
    expect(rows.map((r) => r.refId)).toEqual(["op-error:oe-2"]);
    expect(events).toEqual(["op-error:op-error:oe-1"]);
  });
});
