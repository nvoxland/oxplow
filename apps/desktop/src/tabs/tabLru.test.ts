import { describe, expect, test } from "bun:test";
import { dropFromMru, MAX_PAGE_TABS, selectLruEvictions, touchMru } from "./tabLru.js";

describe("touchMru", () => {
  test("moves an id to the front (most-recently-used)", () => {
    expect(touchMru(["a", "b", "c"], "c")).toEqual(["c", "a", "b"]);
  });
  test("dedupes — an already-present id only appears once", () => {
    expect(touchMru(["b", "a", "b"], "a")).toEqual(["a", "b"]);
  });
  test("no-op (same ref) when already at the front", () => {
    const mru = ["a", "b"];
    expect(touchMru(mru, "a")).toBe(mru);
  });
  test("adds a brand-new id at the front", () => {
    expect(touchMru(["a"], "z")).toEqual(["z", "a"]);
  });
});

describe("dropFromMru", () => {
  test("removes the id", () => {
    expect(dropFromMru(["a", "b", "c"], "b")).toEqual(["a", "c"]);
  });
  test("no-op (same ref) when absent", () => {
    const mru = ["a", "b"];
    expect(dropFromMru(mru, "z")).toBe(mru);
  });
});

describe("selectLruEvictions", () => {
  test("nothing to evict when within the cap", () => {
    expect(selectLruEvictions(["a", "b"], ["b", "a"], { max: 5, protect: [] })).toEqual([]);
  });

  test("evicts the least-recently-used tabs down to the cap", () => {
    // 4 tabs, cap 2 → evict 2. MRU (recent→old): d, c, b, a → LRU-first: a, b.
    const tabs = ["a", "b", "c", "d"];
    const mru = ["d", "c", "b", "a"];
    expect(selectLruEvictions(tabs, mru, { max: 2, protect: [] })).toEqual(["a", "b"]);
  });

  test("never evicts a protected id, skipping to the next LRU candidate", () => {
    const tabs = ["a", "b", "c", "d"];
    const mru = ["d", "c", "b", "a"];
    // a is protected (e.g. active / dirty) → evict b, then c.
    expect(selectLruEvictions(tabs, mru, { max: 2, protect: ["a"] })).toEqual(["b", "c"]);
  });

  test("never-activated tabs (absent from mru) are evicted first", () => {
    const tabs = ["a", "b", "c"];
    // Only a was ever activated; b and c never were → they go before a.
    const mru = ["a"];
    expect(selectLruEvictions(tabs, mru, { max: 1, protect: [] })).toEqual(["b", "c"]);
  });

  test("best-effort: returns fewer than needed when protection blocks eviction", () => {
    const tabs = ["a", "b", "c"];
    const mru = ["c", "b", "a"];
    // Need to drop 2, but a and b are protected → only c is evictable.
    expect(selectLruEvictions(tabs, mru, { max: 1, protect: ["a", "b"] })).toEqual(["c"]);
  });

  test("ignores stale mru ids that are no longer open tabs", () => {
    const tabs = ["a", "b"];
    const mru = ["gone", "b", "a", "also-gone"];
    expect(selectLruEvictions(tabs, mru, { max: 1, protect: [] })).toEqual(["a"]);
  });

  test("the cap constant is 15", () => {
    expect(MAX_PAGE_TABS).toBe(15);
  });
});
