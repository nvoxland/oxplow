import { describe, expect, test } from "bun:test";
import { computeDiffDecorations, diffLineKinds } from "./editor-diff.js";

const lines = (s: string) => s.split("\n");

describe("diffLineKinds", () => {
  test("no change leaves every line unmarked", () => {
    const { kinds, deletedBefore } = diffLineKinds(["a", "b"], ["a", "b"]);
    expect(kinds).toEqual([null, null]);
    expect(deletedBefore.some(Boolean)).toBe(false);
  });

  test("a pure insertion marks only the new lines", () => {
    const { kinds } = diffLineKinds(["a", "c"], ["a", "b", "c"]);
    expect(kinds).toEqual([null, "added", null]);
  });

  test("a replaced line is 'modified', not 'added'", () => {
    const { kinds } = diffLineKinds(["a", "b", "c"], ["a", "B", "c"]);
    expect(kinds).toEqual([null, "modified", null]);
  });

  test("a pure deletion marks the boundary on the surviving line", () => {
    const { kinds, deletedBefore } = diffLineKinds(["a", "b", "c"], ["a", "c"]);
    expect(kinds).toEqual([null, null]);
    expect(deletedBefore[1]).toBe(true);
  });

  // The freeze: the forward walk used to re-derive its alignment by value
  // equality, so a duplicated line let it consume a pair the LCS backtrack
  // had marked as added. It then drifted until both pointers sat on
  // unmarked lines that differed, at which point neither advanced and the
  // renderer's main thread spun forever.
  test("terminates when a duplicate line is appended", () => {
    const { kinds } = diffLineKinds(["a"], ["a", "a"]);
    expect(kinds.filter(Boolean)).toEqual(["added"]);
  });

  test("terminates on duplicate-heavy markdown (blank lines and fences)", () => {
    const before = lines(["# Title", "", "```", "one", "```", "", "## End", ""].join("\n"));
    const after = lines(
      ["# Title", "", "```", "one", "```", "", "### New", "", "```", "two", "```", "", "## End", ""].join("\n"),
    );
    const { kinds } = diffLineKinds(before, after);
    // Six inserted lines: "### New", "", "```", "two", "```", "".
    expect(kinds.filter(Boolean).length).toBe(6);
  });

  test("always terminates on duplicate-heavy random input", () => {
    // Deterministic LCG — duplicate-rich alphabet is the whole point.
    let seed = 0x9e3779b9;
    const rand = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 0x100000000);
    const alphabet = ["", "", "```", "a", "b"];
    for (let trial = 0; trial < 300; trial++) {
      const mk = (len: number) =>
        Array.from({ length: len }, () => alphabet[Math.floor(rand() * alphabet.length)]);
      const oldLines = mk(1 + Math.floor(rand() * 12));
      const newLines = mk(1 + Math.floor(rand() * 12));
      const { kinds, deletedBefore } = diffLineKinds(oldLines, newLines);
      expect(kinds.length).toBe(newLines.length);
      expect(deletedBefore.length).toBe(newLines.length + 1);
    }
  });
});

describe("computeDiffDecorations", () => {
  const monaco = { Range: class { constructor(public a: number, public b: number, public c: number, public d: number) {} } };

  test("emits a gutter class per changed line", () => {
    const decos = computeDiffDecorations(monaco, ["a", "c"], ["a", "b", "c"]);
    expect(decos.map((d) => d.options.linesDecorationsClassName)).toEqual(["oxplow-gutter-added"]);
  });

  test("clamps the deletion marker into the file's line range", () => {
    const decos = computeDiffDecorations(monaco, ["a", "b"], ["b"]);
    const deleted = decos.filter((d) => d.options.linesDecorationsClassName === "oxplow-gutter-deleted");
    expect(deleted.length).toBe(1);
    expect(deleted[0].range.a).toBeGreaterThanOrEqual(1);
    expect(deleted[0].range.a).toBeLessThanOrEqual(1);
  });
});
