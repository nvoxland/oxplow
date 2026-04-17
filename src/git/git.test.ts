import { afterEach, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { detectCurrentBranch, isGitRepo, isGitWorktree, listBranches } from "./git.js";

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
