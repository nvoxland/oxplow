import { describe, expect, test } from "bun:test";
import { layoutCommits } from "./layout.js";
import type { GitLogCommit } from "../../api.js";

function commit(sha: string, parents: string[] = []): GitLogCommit {
  return {
    sha,
    parents: parents.map((p) => ({ sha: p })),
    commit: { author: { name: "", email: "", date: "" }, message: "" },
    refs: [],
  };
}

describe("layoutCommits", () => {
  test("linear history keeps everything in column 0", () => {
    const layout = layoutCommits([commit("c", ["b"]), commit("b", ["a"]), commit("a")]);
    expect(layout.totalColumns).toBe(1);
    expect(layout.rows.map((r) => r.column)).toEqual([0, 0, 0]);
    expect(layout.rows[0]!.fromAbove).toBe(false);
    expect(layout.rows[1]!.fromAbove).toBe(true);
  });

  test("simple merge: branch lane is created then folds back", () => {
    // m -- a
    //   \- b
    //       \- (shared base z)
    //   m: parents [a, b]
    //   a: parent [z]
    //   b: parent [z]
    //   z: no parents
    const layout = layoutCommits([
      commit("m", ["a", "b"]),
      commit("a", ["z"]),
      commit("b", ["z"]),
      commit("z"),
    ]);
    expect(layout.rows[0]!.column).toBe(0); // merge at col 0
    expect(layout.rows[0]!.parentEdges.map((e) => e.toCol)).toEqual([0, 1]);
    expect(layout.rows[1]!.column).toBe(0); // a continues at col 0
    expect(layout.rows[2]!.column).toBe(1); // b on branch lane
    // z is the shared parent: whichever side (a or b) that reaches it first
    // while the other lane still waits determines the column. In our order,
    // `a -> z` is processed first so z is placed at col 0.
    expect(layout.rows[3]!.column).toBe(0);
    expect(layout.totalColumns).toBeGreaterThanOrEqual(2);
  });

  test("dangling parent outside the commit set is marked missing", () => {
    const layout = layoutCommits([commit("c", ["unknown"])]);
    expect(layout.rows[0]!.parentEdges[0]!.missing).toBe(true);
  });
});
