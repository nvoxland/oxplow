import { describe, expect, test } from "bun:test";

import type { WikiRefFreshness } from "../../tauri-bridge/generated/bindings.js";
import { summarizeWikiFreshness } from "./wikiFreshness.js";

function row(path: string, stale: boolean): WikiRefFreshness {
  return {
    path,
    local_snapshot_id: 1,
    closest_git_version: null,
    git_version_exact: false,
    latest_snapshot_id: stale ? 2 : 1,
    stale,
  };
}

describe("summarizeWikiFreshness", () => {
  test("no refs → fresh with zero counts", () => {
    expect(summarizeWikiFreshness([])).toEqual({
      totalRefs: 0,
      staleRefs: [],
      freshness: "fresh",
    });
  });

  test("all refs current → fresh", () => {
    const s = summarizeWikiFreshness([row("a.rs", false), row("b.rs", false)]);
    expect(s.freshness).toBe("fresh");
    expect(s.totalRefs).toBe(2);
    expect(s.staleRefs).toEqual([]);
  });

  test("some refs stale → stale, listing the stale paths", () => {
    const s = summarizeWikiFreshness([row("a.rs", true), row("b.rs", false), row("c.rs", true)]);
    expect(s.freshness).toBe("stale");
    expect(s.staleRefs).toEqual(["a.rs", "c.rs"]);
    expect(s.totalRefs).toBe(3);
  });

  test("every ref stale → very-stale", () => {
    const s = summarizeWikiFreshness([row("a.rs", true), row("b.rs", true)]);
    expect(s.freshness).toBe("very-stale");
    expect(s.staleRefs).toEqual(["a.rs", "b.rs"]);
  });
});
