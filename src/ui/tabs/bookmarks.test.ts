import { describe, expect, test } from "bun:test";
import { createBookmarksStore } from "./bookmarks.js";
import type { TabRef } from "./tabState.js";

function memStorage() {
  const map = new Map<string, string>();
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => { map.set(k, v); },
    removeItem: (k: string) => { map.delete(k); },
    _map: map,
  };
}

const REF_A: TabRef = { id: "git-dashboard", kind: "git-dashboard", payload: null };
const REF_B: TabRef = { id: "wi:wi-7", kind: "work-item", payload: { itemId: "wi-7" } };

describe("bookmarks store", () => {
  test("starts empty", () => {
    const s = createBookmarksStore(memStorage());
    expect(s.bookmarks("t-1", "s-1")).toEqual([]);
    expect(s.isBookmarked("t-1", "s-1", REF_A.id)).toBe(false);
  });

  test("add/remove at thread scope", () => {
    const s = createBookmarksStore(memStorage());
    s.add("thread", "t-1", "s-1", REF_A, "Git Dashboard");
    expect(s.isBookmarked("t-1", "s-1", REF_A.id)).toBe(true);
    expect(s.scopesFor("t-1", "s-1", REF_A.id)).toEqual(["thread"]);
    s.remove("thread", "t-1", "s-1", REF_A.id);
    expect(s.isBookmarked("t-1", "s-1", REF_A.id)).toBe(false);
  });

  test("dedup across scopes — same ref bookmarked at thread and global appears once", () => {
    const s = createBookmarksStore(memStorage());
    s.add("global", null, null, REF_A);
    s.add("thread", "t-1", "s-1", REF_A);
    const list = s.bookmarks("t-1", "s-1");
    expect(list).toHaveLength(1);
    expect(s.scopesFor("t-1", "s-1", REF_A.id).sort()).toEqual(["global", "thread"]);
  });

  test("stream bookmarks isolate by streamId", () => {
    const s = createBookmarksStore(memStorage());
    s.add("stream", null, "s-1", REF_A);
    expect(s.bookmarks("t-1", "s-1").map((b) => b.ref.id)).toEqual([REF_A.id]);
    expect(s.bookmarks("t-1", "s-2")).toEqual([]);
  });

  test("global bookmarks visible from any stream/thread", () => {
    const s = createBookmarksStore(memStorage());
    s.add("global", null, null, REF_A);
    expect(s.bookmarks("t-1", "s-1").map((b) => b.ref.id)).toEqual([REF_A.id]);
    expect(s.bookmarks("t-9", "s-9").map((b) => b.ref.id)).toEqual([REF_A.id]);
  });

  test("merged list sorted by addedAt descending", async () => {
    const s = createBookmarksStore(memStorage());
    s.add("global", null, null, REF_A);
    await new Promise((r) => setTimeout(r, 5));
    s.add("thread", "t-1", "s-1", REF_B);
    const list = s.bookmarks("t-1", "s-1");
    expect(list.map((b) => b.ref.id)).toEqual([REF_B.id, REF_A.id]);
  });

  test("lastScope persists and defaults to thread", () => {
    const storage = memStorage();
    const s = createBookmarksStore(storage);
    expect(s.lastScope()).toBe("thread");
    s.setLastScope("global");
    const s2 = createBookmarksStore(storage);
    expect(s2.lastScope()).toBe("global");
  });

  test("add is idempotent within a scope", () => {
    const s = createBookmarksStore(memStorage());
    s.add("global", null, null, REF_A);
    s.add("global", null, null, REF_A);
    expect(s.bookmarks(null, null)).toHaveLength(1);
  });

  test("subscribe fires on add/remove", () => {
    const s = createBookmarksStore(memStorage());
    let calls = 0;
    const unsub = s.subscribe(() => { calls += 1; });
    s.add("global", null, null, REF_A);
    s.remove("global", null, null, REF_A.id);
    expect(calls).toBe(2);
    unsub();
    s.add("global", null, null, REF_B);
    expect(calls).toBe(2);
  });
});
