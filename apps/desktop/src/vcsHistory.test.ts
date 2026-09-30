import { describe, expect, test } from "bun:test";

import type { RevisionInfo } from "./tauri-bridge/generated/bindings.js";
import { topoOrder } from "./vcsHistory.js";

function commit(id: string, time: number, parents: string[] = []): RevisionInfo {
  return { id, short_id: id, author: "", email: "", time, subject: id, parents };
}

describe("topoOrder", () => {
  test("a child comes before its parent even when made in the same second", () => {
    // Read time-desc with a sha tiebreak: the parent `a` sorts first.
    const rows = [commit("a", 10), commit("b", 10, ["a"]), commit("c", 10, ["b"])];
    expect(topoOrder(rows).map((c) => c.id)).toEqual(["c", "b", "a"]);
  });

  test("clock skew: a child dated before its parent still comes first", () => {
    const rows = [commit("p", 20), commit("k", 5, ["p"])];
    expect(topoOrder(rows).map((c) => c.id)).toEqual(["k", "p"]);
  });

  test("otherwise newest first, merges after both sides", () => {
    const rows = [
      commit("m", 30, ["x", "y"]),
      commit("y", 25, ["base"]),
      commit("x", 20, ["base"]),
      commit("base", 10),
      commit("other", 15),
    ];
    expect(topoOrder(rows).map((c) => c.id)).toEqual(["m", "y", "x", "other", "base"]);
  });
});
