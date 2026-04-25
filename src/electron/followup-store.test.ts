import { describe, expect, test } from "bun:test";
import { FollowupStore } from "./followup-store.js";

describe("FollowupStore", () => {
  test("add returns an id and surfaces in list", () => {
    const store = new FollowupStore();
    const entry = store.add("t-1", "wire up the redo button");
    expect(entry.id).toMatch(/^fu-/);
    expect(entry.note).toBe("wire up the redo button");
    expect(store.list("t-1")).toHaveLength(1);
    expect(store.list("t-1")[0]!.id).toBe(entry.id);
  });

  test("list isolates threads", () => {
    const store = new FollowupStore();
    store.add("t-1", "alpha");
    store.add("t-2", "beta");
    expect(store.list("t-1").map((f) => f.note)).toEqual(["alpha"]);
    expect(store.list("t-2").map((f) => f.note)).toEqual(["beta"]);
    expect(store.list("t-empty")).toEqual([]);
  });

  test("remove drops the entry by id and reports whether it existed", () => {
    const store = new FollowupStore();
    const a = store.add("t-1", "a");
    store.add("t-1", "b");
    expect(store.remove("t-1", a.id)).toBe(true);
    expect(store.list("t-1")).toHaveLength(1);
    expect(store.remove("t-1", a.id)).toBe(false);
  });

  test("rejects empty notes", () => {
    const store = new FollowupStore();
    expect(() => store.add("t-1", "")).toThrow();
    expect(() => store.add("t-1", "   ")).toThrow();
  });

  test("subscribe fires for add/remove with the right kind", () => {
    const store = new FollowupStore();
    const seen: Array<{ kind: string; id: string | null; threadId: string }> = [];
    const off = store.subscribe((change) => seen.push({ kind: change.kind, id: change.id, threadId: change.threadId }));
    const a = store.add("t-1", "x");
    store.remove("t-1", a.id);
    off();
    store.add("t-1", "after-unsubscribe");
    expect(seen).toEqual([
      { kind: "added", id: a.id, threadId: "t-1" },
      { kind: "removed", id: a.id, threadId: "t-1" },
    ]);
  });

  test("clear wipes a thread's followups and emits cleared", () => {
    const store = new FollowupStore();
    store.add("t-1", "a");
    store.add("t-1", "b");
    const seen: string[] = [];
    store.subscribe((change) => seen.push(change.kind));
    store.clear("t-1");
    expect(store.list("t-1")).toEqual([]);
    expect(seen).toEqual(["cleared"]);
    // Idempotent — second clear on empty thread is silent.
    store.clear("t-1");
    expect(seen).toEqual(["cleared"]);
  });
});
