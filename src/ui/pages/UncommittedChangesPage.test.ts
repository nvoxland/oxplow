import { describe, expect, test } from "bun:test";
import type { BranchChangeEntry } from "../api.js";
import { buildTree, summarize } from "./UncommittedChangesPage.js";

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

describe("buildTree", () => {
  test("groups files into nested directory nodes", () => {
    const tree = buildTree([
      entry("src/git/git.ts", "modified", 3, 1),
      entry("src/git/git.test.ts", "modified", 5, 0),
      entry("src/ui/App.tsx", "modified", 2, 2),
      entry("README.md", "added", 1, 0),
    ]);
    // root totals reflect every file
    expect(tree.totalFiles).toBe(4);
    expect(tree.totalAdditions).toBe(11);
    expect(tree.totalDeletions).toBe(3);
    // root has README.md as a direct file
    expect(tree.files.map((f) => f.path)).toEqual(["README.md"]);
    // src/ folder rolls up its three files
    const src = tree.children.get("src");
    expect(src).toBeDefined();
    expect(src!.totalFiles).toBe(3);
    expect(src!.totalAdditions).toBe(10);
    expect(src!.totalDeletions).toBe(3);
    // src/git nested with two files
    const srcGit = src!.children.get("git");
    expect(srcGit).toBeDefined();
    expect(srcGit!.totalFiles).toBe(2);
    expect(srcGit!.totalAdditions).toBe(8);
    expect(srcGit!.files.map((f) => f.path).sort()).toEqual([
      "src/git/git.test.ts",
      "src/git/git.ts",
    ]);
    // src/ui has only App.tsx
    const srcUi = src!.children.get("ui");
    expect(srcUi).toBeDefined();
    expect(srcUi!.totalFiles).toBe(1);
    expect(srcUi!.files.map((f) => f.path)).toEqual(["src/ui/App.tsx"]);
  });

  test("handles untracked files (null additions/deletions) without NaN totals", () => {
    const tree = buildTree([
      entry("docs/new.md", "untracked"),
      entry("docs/edit.md", "modified", 4, 1),
    ]);
    const docs = tree.children.get("docs");
    expect(docs).toBeDefined();
    expect(docs!.totalFiles).toBe(2);
    expect(docs!.totalAdditions).toBe(4);
    expect(docs!.totalDeletions).toBe(1);
  });

  test("empty input returns an empty root", () => {
    const tree = buildTree([]);
    expect(tree.totalFiles).toBe(0);
    expect(tree.children.size).toBe(0);
    expect(tree.files).toEqual([]);
  });

  test("only folders with changes appear in the tree", () => {
    const tree = buildTree([entry("a/b/c.ts", "modified", 1, 0)]);
    expect([...tree.children.keys()]).toEqual(["a"]);
    const a = tree.children.get("a")!;
    expect([...a.children.keys()]).toEqual(["b"]);
    const b = a.children.get("b")!;
    expect(b.files.map((f) => f.path)).toEqual(["a/b/c.ts"]);
  });
});
