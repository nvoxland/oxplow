import { describe, expect, test } from "bun:test";
import type { BranchChangeEntry } from "../tauri-bridge/index.js";
import { summarize, totalsLabel } from "./UncommittedChangesPage.js";

function entry(
  path: string,
  status: BranchChangeEntry["status"],
  additions: number | null = null,
  deletions: number | null = null,
): BranchChangeEntry {
  return { path, status, additions, deletions };
}

describe("summarize", () => {
  test("counts each status bucket independently", () => {
    const result = summarize([
      entry("a.ts", "modified", 5, 2),
      entry("b.ts", "added", 10, 0),
      entry("c.ts", "deleted", 0, 8),
      entry("d.ts", "renamed", 1, 1),
      entry("e.ts", "untracked"),
    ]);
    expect(result.total).toBe(5);
    expect(result.modified).toBe(1);
    expect(result.added).toBe(1);
    expect(result.deleted).toBe(1);
    expect(result.renamed).toBe(1);
    expect(result.untracked).toBe(1);
  });

  test("sums additions and deletions across files; nulls treated as 0", () => {
    const result = summarize([
      entry("a.ts", "modified", 5, 2),
      entry("b.ts", "added", 10, 0),
      entry("c.ts", "untracked"), // additions/deletions null
    ]);
    expect(result.additions).toBe(15);
    expect(result.deletions).toBe(2);
  });

  test("empty input is all zeros", () => {
    const result = summarize([]);
    expect(result.total).toBe(0);
    expect(result.additions).toBe(0);
    expect(result.deletions).toBe(0);
  });
});

test("totalsLabel reads the file count and line totals", () => {
  expect(totalsLabel(summarize([entry("a.ts", "modified", 5, 2), entry("b.ts", "added", 10, null)]))).toBe(
    "2 changed · +15 −2",
  );
});
