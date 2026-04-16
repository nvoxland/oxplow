import { afterEach, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { listGitStatuses } from "./git.js";
import {
  listWorkspaceEntries,
  listWorkspaceFiles,
  readWorkspaceFile,
  summarizeGitStatuses,
  createWorkspaceDirectory,
  createWorkspaceFile,
  deleteWorkspacePath,
  renameWorkspacePath,
  writeWorkspaceFile,
} from "./workspace-files.js";

const tempDirs: string[] = [];

afterEach(() => {
  while (tempDirs.length > 0) {
    const dir = tempDirs.pop();
    if (dir) rmSync(dir, { recursive: true, force: true });
  }
});

test("listWorkspaceEntries sorts directories first and propagates descendant git changes", () => {
  const root = mkProjectDir();
  mkdirSync(join(root, "src"), { recursive: true });
  writeFileSync(join(root, "src", "index.ts"), "console.log('hi')\n", "utf8");
  writeFileSync(join(root, "README.md"), "# hello\n", "utf8");

  const entries = listWorkspaceEntries(root, "", new Map([["src/index.ts", "modified"]]));

  expect(entries).toEqual([
    { name: "src", path: "src", kind: "directory", gitStatus: null, hasChanges: true },
    { name: "README.md", path: "README.md", kind: "file", gitStatus: null, hasChanges: false },
  ]);
});

test("readWorkspaceFile returns content and rejects path traversal", () => {
  const root = mkProjectDir();
  writeFileSync(join(root, "notes.txt"), "hello\n", "utf8");

  expect(readWorkspaceFile(root, "notes.txt")).toEqual({ path: "notes.txt", content: "hello\n" });
  expect(() => readWorkspaceFile(root, "../secret.txt")).toThrow(/outside workspace/);
});

test("writeWorkspaceFile updates an existing file and rejects path traversal", () => {
  const root = mkProjectDir();
  writeFileSync(join(root, "notes.txt"), "before\n", "utf8");

  expect(writeWorkspaceFile(root, "notes.txt", "after\n")).toEqual({ path: "notes.txt", content: "after\n" });
  expect(readWorkspaceFile(root, "notes.txt").content).toBe("after\n");
  expect(() => writeWorkspaceFile(root, "../secret.txt", "bad")).toThrow(/outside workspace/);
});

test("createWorkspaceFile creates a new file and rejects duplicates", () => {
  const root = mkProjectDir();
  expect(createWorkspaceFile(root, "notes.txt", "hello\n")).toEqual({ path: "notes.txt", content: "hello\n" });
  expect(readWorkspaceFile(root, "notes.txt").content).toBe("hello\n");
  expect(() => createWorkspaceFile(root, "notes.txt", "again")).toThrow(/already exists/);
});

test("createWorkspaceDirectory creates a new directory", () => {
  const root = mkProjectDir();
  expect(createWorkspaceDirectory(root, "src/utils")).toEqual({ path: "src/utils" });
  expect(existsSync(join(root, "src", "utils"))).toBe(true);
});

test("renameWorkspacePath renames files within the workspace", () => {
  const root = mkProjectDir();
  writeFileSync(join(root, "notes.txt"), "before\n", "utf8");
  expect(renameWorkspacePath(root, "notes.txt", "renamed.txt")).toEqual({
    fromPath: "notes.txt",
    toPath: "renamed.txt",
  });
  expect(existsSync(join(root, "notes.txt"))).toBe(false);
  expect(readWorkspaceFile(root, "renamed.txt").content).toBe("before\n");
});

test("deleteWorkspacePath removes files and directories", () => {
  const root = mkProjectDir();
  mkdirSync(join(root, "src"), { recursive: true });
  writeFileSync(join(root, "src", "notes.txt"), "before\n", "utf8");
  expect(deleteWorkspacePath(root, "src/notes.txt")).toEqual({ path: "src/notes.txt" });
  expect(existsSync(join(root, "src", "notes.txt"))).toBe(false);
  expect(deleteWorkspacePath(root, "src")).toEqual({ path: "src" });
  expect(existsSync(join(root, "src"))).toBe(false);
});

test("listGitStatuses reports modified and untracked files", () => {
  const root = mkGitRepo();
  writeFileSync(join(root, "tracked.txt"), "changed\n", "utf8");
  writeFileSync(join(root, "new.txt"), "new\n", "utf8");

  const statuses = listGitStatuses(root);
  expect(statuses.get("tracked.txt")).toBe("modified");
  expect(statuses.get("new.txt")).toBe("untracked");
});

test("listWorkspaceFiles returns recursive file results with git statuses", () => {
  const root = mkProjectDir();
  mkdirSync(join(root, "src"), { recursive: true });
  writeFileSync(join(root, "src", "index.ts"), "console.log('hi')\n", "utf8");
  writeFileSync(join(root, "README.md"), "# hello\n", "utf8");

  const files = listWorkspaceFiles(root, new Map([
    ["src/index.ts", "modified"],
    ["README.md", "untracked"],
  ]));

  expect(files).toEqual([
    { path: "README.md", gitStatus: "untracked" },
    { path: "src/index.ts", gitStatus: "modified" },
  ]);
});

test("summarizeGitStatuses returns counts by status", () => {
  const summary = summarizeGitStatuses(new Map([
    ["a.ts", "modified"],
    ["b.ts", "modified"],
    ["c.ts", "untracked"],
  ]));

  expect(summary).toEqual({
    modified: 2,
    added: 0,
    deleted: 0,
    renamed: 0,
    untracked: 1,
    total: 3,
  });
});

function mkProjectDir(): string {
  const dir = mkdtempSync(join(tmpdir(), "newde-workspace-files-"));
  tempDirs.push(dir);
  return dir;
}

function mkGitRepo(): string {
  const dir = mkProjectDir();
  execFileSync("git", ["init"], { cwd: dir, stdio: "ignore" });
  execFileSync("git", ["config", "user.email", "test@example.com"], { cwd: dir, stdio: "ignore" });
  execFileSync("git", ["config", "user.name", "Test User"], { cwd: dir, stdio: "ignore" });
  writeFileSync(join(dir, "tracked.txt"), "initial\n", "utf8");
  execFileSync("git", ["add", "tracked.txt"], { cwd: dir, stdio: "ignore" });
  execFileSync("git", ["commit", "-m", "init"], { cwd: dir, stdio: "ignore" });
  return dir;
}
