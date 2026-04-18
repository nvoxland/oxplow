import { afterEach, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  detectBaseBranch,
  detectCurrentBranch,
  isGitRepo,
  isGitWorktree,
  listBranchChanges,
  listBranches,
  readFileAtRef,
} from "./git.js";

const tempDirs: string[] = [];

afterEach(() => {
  while (tempDirs.length > 0) {
    const dir = tempDirs.pop();
    if (dir) rmSync(dir, { recursive: true, force: true });
  }
});

test("isGitRepo only returns true for the repo root, not nested directories", () => {
  const repoDir = mkRepo();
  const nestedDir = join(repoDir, "nested", "child");
  mkdirSync(nestedDir, { recursive: true });

  expect(isGitRepo(repoDir)).toBe(true);
  expect(isGitRepo(nestedDir)).toBe(false);
});

test("detectCurrentBranch returns null outside the repo root", () => {
  const repoDir = mkRepo();
  const nestedDir = join(repoDir, "nested");
  mkdirSync(nestedDir, { recursive: true });

  expect(detectCurrentBranch(repoDir)).toBe("main");
  expect(detectCurrentBranch(nestedDir)).toBeNull();
});

test("isGitWorktree distinguishes secondary worktrees (.git is a file) from the main checkout", () => {
  const repoDir = mkRepo();
  expect(isGitWorktree(repoDir)).toBe(false);

  const worktreeDir = mkdtempSync(join(tmpdir(), "newde-git-worktree-"));
  tempDirs.push(worktreeDir);
  rmSync(worktreeDir, { recursive: true, force: true });
  execFileSync("git", ["-C", repoDir, "worktree", "add", worktreeDir, "-b", "feature"], { stdio: "ignore" });
  expect(isGitWorktree(worktreeDir)).toBe(true);
  expect(isGitWorktree(repoDir)).toBe(false);
});

test("isGitWorktree returns false for directories without a .git entry", () => {
  const dir = mkdtempSync(join(tmpdir(), "newde-plain-"));
  tempDirs.push(dir);
  expect(isGitWorktree(dir)).toBe(false);
});

test("listBranches is empty outside the repo root", () => {
  const repoDir = mkRepo();
  const nestedDir = join(repoDir, "nested");
  mkdirSync(nestedDir, { recursive: true });

  expect(listBranches(repoDir).length).toBeGreaterThan(0);
  expect(listBranches(nestedDir)).toEqual([]);
});

test("listBranchChanges returns committed and uncommitted diffs vs merge base", () => {
  const repoDir = mkRepo();
  // Create a feature branch at HEAD so main and feature share the same base.
  execFileSync("git", ["-C", repoDir, "checkout", "-b", "feature"], { stdio: "ignore" });
  writeFileSync(join(repoDir, "committed.txt"), "hello\n", "utf8");
  execFileSync("git", ["-C", repoDir, "add", "committed.txt"], { stdio: "ignore" });
  execFileSync("git", ["-C", repoDir, "commit", "-m", "add committed"], { stdio: "ignore" });
  writeFileSync(join(repoDir, "wip.txt"), "wip\n", "utf8");
  writeFileSync(join(repoDir, "untracked.txt"), "nope\n", "utf8");

  const result = listBranchChanges(repoDir, "main");
  expect(result.mergeBase).not.toBeNull();
  const paths = result.files.map((f) => f.path).sort();
  expect(paths).toEqual(["committed.txt", "untracked.txt", "wip.txt"]);
  const committed = result.files.find((f) => f.path === "committed.txt");
  expect(committed?.status).toBe("added");
  expect(committed?.additions).toBe(1);
  const untracked = result.files.find((f) => f.path === "untracked.txt");
  expect(untracked?.status).toBe("untracked");
});

test("readFileAtRef returns the ref content; null when the file is absent", () => {
  const repoDir = mkRepo();
  const initial = readFileAtRef(repoDir, "HEAD", "README.md");
  expect(initial).toBe("# test\n");
  expect(readFileAtRef(repoDir, "HEAD", "does-not-exist.md")).toBeNull();
});

test("detectBaseBranch prefers main when no origin is configured", () => {
  const repoDir = mkRepo();
  expect(detectBaseBranch(repoDir)).toBe("main");
});

function mkRepo(): string {
  const dir = mkdtempSync(join(tmpdir(), "newde-git-test-"));
  tempDirs.push(dir);
  execFileSync("git", ["init", "-b", "main", dir], { stdio: "ignore" });
  execFileSync("git", ["-C", dir, "config", "user.name", "Test User"], { stdio: "ignore" });
  execFileSync("git", ["-C", dir, "config", "user.email", "test@example.com"], { stdio: "ignore" });
  writeFileSync(join(dir, "README.md"), "# test\n", "utf8");
  execFileSync("git", ["-C", dir, "add", "README.md"], { stdio: "ignore" });
  execFileSync("git", ["-C", dir, "commit", "-m", "init"], { stdio: "ignore" });
  return dir;
}
