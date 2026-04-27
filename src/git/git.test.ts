import { afterEach, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  detectBaseBranch,
  detectCurrentBranch,
  getAheadBehind,
  getCommitDetail,
  getCommitsAheadOf,
  gitBlame,
  gitMerge,
  gitPushCurrentTo,
  isGitRepo,
  isGitWorktree,
  listBranchChanges,
  listBranches,
  listExistingWorktrees,
  listRecentRemoteBranches,
  parseBlamePorcelain,
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

  const worktreeDir = mkdtempSync(join(tmpdir(), "oxplow-git-worktree-"));
  tempDirs.push(worktreeDir);
  rmSync(worktreeDir, { recursive: true, force: true });
  execFileSync("git", ["-C", repoDir, "worktree", "add", worktreeDir, "-b", "feature"], { stdio: "ignore" });
  expect(isGitWorktree(worktreeDir)).toBe(true);
  expect(isGitWorktree(repoDir)).toBe(false);
});

test("isGitWorktree returns false for directories without a .git entry", () => {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-plain-"));
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

test("parseBlamePorcelain handles repeated-sha hunks by reusing cached metadata", () => {
  const raw = [
    "abcd1234abcd1234abcd1234abcd1234abcd1234 1 1 2",
    "author Alice",
    "author-mail <alice@example.com>",
    "author-time 1700000000",
    "author-tz +0000",
    "committer Alice",
    "summary first commit",
    "filename foo.txt",
    "\thello",
    "abcd1234abcd1234abcd1234abcd1234abcd1234 2 2",
    "\tworld",
    "0000000000000000000000000000000000000000 3 3 1",
    "author Not Committed Yet",
    "author-mail <not.committed.yet@example.com>",
    "author-time 1710000000",
    "author-tz +0000",
    "committer Not Committed Yet",
    "summary Version of foo.txt from foo.txt",
    "filename foo.txt",
    "\twip",
    "",
  ].join("\n");
  const parsed = parseBlamePorcelain(raw);
  expect(parsed.length).toBe(3);
  expect(parsed[0]).toMatchObject({
    line: 1,
    sha: "abcd1234abcd1234abcd1234abcd1234abcd1234",
    author: "Alice",
    authorMail: "alice@example.com",
    authorTime: 1700000000,
    summary: "first commit",
  });
  expect(parsed[1]).toMatchObject({ line: 2, sha: "abcd1234abcd1234abcd1234abcd1234abcd1234", author: "Alice" });
  expect(parsed[2]?.sha).toBe("0000000000000000000000000000000000000000");
});

test("gitBlame returns one BlameLine per final-file line with commit metadata", () => {
  const repoDir = mkRepo();
  writeFileSync(join(repoDir, "foo.txt"), "one\ntwo\n", "utf8");
  execFileSync("git", ["-C", repoDir, "add", "foo.txt"], { stdio: "ignore" });
  execFileSync("git", ["-C", repoDir, "commit", "-m", "add foo"], { stdio: "ignore" });

  const blame = gitBlame(repoDir, "foo.txt");
  expect(blame.length).toBe(2);
  expect(blame[0]?.line).toBe(1);
  expect(blame[1]?.line).toBe(2);
  expect(blame[0]?.author).toBe("Test User");
  expect(blame[0]?.summary).toBe("add foo");
  expect(blame[0]?.sha).toMatch(/^[0-9a-f]{40}$/);
});

test("gitBlame returns [] for non-git paths", () => {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-noblame-"));
  tempDirs.push(dir);
  expect(gitBlame(dir, "nope.txt")).toEqual([]);
});

test("getCommitDetail returns files with per-file additions and deletions", () => {
  const repoDir = mkRepo();
  writeFileSync(join(repoDir, "a.txt"), "one\ntwo\nthree\n", "utf8");
  writeFileSync(join(repoDir, "b.txt"), "bee\n", "utf8");
  execFileSync("git", ["-C", repoDir, "add", "a.txt", "b.txt"], { stdio: "ignore" });
  execFileSync("git", ["-C", repoDir, "commit", "-m", "add files\n\nbody line"], { stdio: "ignore" });
  const sha = execFileSync("git", ["-C", repoDir, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();

  const detail = getCommitDetail(repoDir, sha);
  expect(detail).not.toBeNull();
  expect(detail!.files.length).toBe(2);
  const a = detail!.files.find((f) => f.path === "a.txt");
  const b = detail!.files.find((f) => f.path === "b.txt");
  expect(a).toBeDefined();
  expect(a!.additions).toBe(3);
  expect(a!.deletions).toBe(0);
  expect(a!.status).toBe("added");
  expect(b).toBeDefined();
  expect(b!.additions).toBe(1);
  expect(b!.deletions).toBe(0);
  expect(detail!.subject).toBe("add files");
  expect(detail!.body).toBe("body line");
});

test("detectBaseBranch prefers main when no origin is configured", () => {
  const repoDir = mkRepo();
  expect(detectBaseBranch(repoDir)).toBe("main");
});

test("getAheadBehind reports commits diverged between base and head", () => {
  const repoDir = mkRepo();
  // Initial commit is on main. Branch off, add 2 commits on feature; add 1 on main.
  execFileSync("git", ["-C", repoDir, "checkout", "-b", "feature"], { stdio: "ignore" });
  commitFile(repoDir, "f1.txt", "1\n", "feature 1");
  commitFile(repoDir, "f2.txt", "2\n", "feature 2");
  execFileSync("git", ["-C", repoDir, "checkout", "main"], { stdio: "ignore" });
  commitFile(repoDir, "m1.txt", "1\n", "main 1");

  const result = getAheadBehind(repoDir, "main", "feature");
  // base=main, head=feature: ahead = commits in feature not in main, behind = commits in main not in feature
  expect(result).toEqual({ ahead: 2, behind: 1 });
});

test("getAheadBehind defaults head to HEAD", () => {
  const repoDir = mkRepo();
  execFileSync("git", ["-C", repoDir, "checkout", "-b", "feature"], { stdio: "ignore" });
  commitFile(repoDir, "f1.txt", "1\n", "feature 1");
  const result = getAheadBehind(repoDir, "main");
  expect(result).toEqual({ ahead: 1, behind: 0 });
});

test("getCommitsAheadOf returns commits in head not in base, newest first", () => {
  const repoDir = mkRepo();
  execFileSync("git", ["-C", repoDir, "checkout", "-b", "feature"], { stdio: "ignore" });
  commitFile(repoDir, "a.txt", "a\n", "feat a");
  commitFile(repoDir, "b.txt", "b\n", "feat b");

  const commits = getCommitsAheadOf(repoDir, "main", "feature");
  expect(commits.length).toBe(2);
  // git log default is newest first, so "feat b" comes before "feat a"
  expect(commits[0]?.commit.message).toBe("feat b");
  expect(commits[1]?.commit.message).toBe("feat a");
});

test("listRecentRemoteBranches sorts by committer date, newest first", () => {
  const remoteDir = mkBareRemote();
  const repoDir = mkRepo();
  execFileSync("git", ["-C", repoDir, "remote", "add", "origin", remoteDir], { stdio: "ignore" });
  // Push main, then create branches with different commit times.
  execFileSync("git", ["-C", repoDir, "push", "origin", "main"], { stdio: "ignore" });

  execFileSync("git", ["-C", repoDir, "checkout", "-b", "older"], { stdio: "ignore" });
  commitFile(repoDir, "older.txt", "x\n", "older commit", { date: "2024-01-01T00:00:00Z" });
  execFileSync("git", ["-C", repoDir, "push", "origin", "older"], { stdio: "ignore" });

  execFileSync("git", ["-C", repoDir, "checkout", "main"], { stdio: "ignore" });
  execFileSync("git", ["-C", repoDir, "checkout", "-b", "newer"], { stdio: "ignore" });
  commitFile(repoDir, "newer.txt", "x\n", "newer commit", { date: "2025-06-01T00:00:00Z" });
  execFileSync("git", ["-C", repoDir, "push", "origin", "newer"], { stdio: "ignore" });

  const branches = listRecentRemoteBranches(repoDir, 10);
  const names = branches.map((b) => b.shortName);
  // origin/HEAD is filtered out; newer branches first
  expect(names).toContain("origin/newer");
  expect(names).toContain("origin/older");
  expect(names.indexOf("origin/newer")).toBeLessThan(names.indexOf("origin/older"));
  const newer = branches.find((b) => b.shortName === "origin/newer");
  expect(newer?.remote).toBe("origin");
  expect(newer?.branch).toBe("newer");
  expect(newer?.lastCommitSubject).toBe("newer commit");
});

test("gitPushCurrentTo pushes HEAD into a named remote branch without touching local working dir", () => {
  const remoteDir = mkBareRemote();
  const repoDir = mkRepo();
  execFileSync("git", ["-C", repoDir, "remote", "add", "origin", remoteDir], { stdio: "ignore" });
  commitFile(repoDir, "ship.txt", "shipping\n", "ship it");

  // Capture working dir state before push.
  const statusBefore = execFileSync("git", ["-C", repoDir, "status", "--porcelain"], { encoding: "utf8" });
  const headBefore = execFileSync("git", ["-C", repoDir, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();

  const result = gitPushCurrentTo(repoDir, "origin", "release-target");
  expect(result.ok).toBe(true);

  // Remote ref was created and points at our HEAD.
  const remoteRef = execFileSync("git", ["-C", remoteDir, "rev-parse", "refs/heads/release-target"], {
    encoding: "utf8",
  }).trim();
  expect(remoteRef).toBe(headBefore);

  // Local working dir is untouched.
  const statusAfter = execFileSync("git", ["-C", repoDir, "status", "--porcelain"], { encoding: "utf8" });
  expect(statusAfter).toBe(statusBefore);
});

test("gitMerge into current branch does not touch a sibling worktree's working dir", () => {
  const repoDir = mkRepo();
  // Create a sibling worktree on branch "sibling" with a dirty working file.
  const siblingDir = mkdtempSync(join(tmpdir(), "oxplow-sibling-"));
  tempDirs.push(siblingDir);
  rmSync(siblingDir, { recursive: true, force: true });
  execFileSync("git", ["-C", repoDir, "worktree", "add", "-b", "sibling", siblingDir], { stdio: "ignore" });
  // Leave the sibling worktree with an uncommitted file present.
  writeFileSync(join(siblingDir, "wip.txt"), "in progress\n", "utf8");

  // Capture sibling state before the merge (status + file content).
  const siblingStatusBefore = execFileSync("git", ["-C", siblingDir, "status", "--porcelain"], {
    encoding: "utf8",
  });
  const siblingWipBefore = readFileSync(join(siblingDir, "wip.txt"), "utf8");

  // From the sibling worktree, advance its branch with a commit so the primary has something to merge in.
  commitFileIn(siblingDir, "advance.txt", "advanced\n", "advance sibling");

  // Re-capture sibling state after the commit but before primary merge.
  const siblingHeadAdvanced = execFileSync("git", ["-C", siblingDir, "rev-parse", "HEAD"], {
    encoding: "utf8",
  }).trim();

  // Merge sibling into the primary worktree's current branch.
  const result = gitMerge(repoDir, "sibling");
  expect(result.ok).toBe(true);

  // Primary branch should now contain the sibling's tip commit.
  const primaryHead = execFileSync("git", ["-C", repoDir, "rev-parse", "HEAD"], {
    encoding: "utf8",
  }).trim();
  // After a fast-forward or merge, sibling's commit is reachable from primary HEAD.
  const reachable = execFileSync("git", ["-C", repoDir, "merge-base", "--is-ancestor", siblingHeadAdvanced, primaryHead], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  expect(reachable).toBe("");

  // Sibling worktree should be byte-identical to its post-commit state — wip.txt unchanged, no surprise modifications.
  const siblingHeadAfter = execFileSync("git", ["-C", siblingDir, "rev-parse", "HEAD"], {
    encoding: "utf8",
  }).trim();
  const siblingStatusAfter = execFileSync("git", ["-C", siblingDir, "status", "--porcelain"], {
    encoding: "utf8",
  });
  const siblingWipAfter = readFileSync(join(siblingDir, "wip.txt"), "utf8");

  expect(siblingHeadAfter).toBe(siblingHeadAdvanced);
  expect(siblingWipAfter).toBe(siblingWipBefore);
  // Status diff: only the new committed `advance.txt` should differ from the pre-commit baseline; wip.txt remains untracked.
  expect(siblingStatusAfter).toContain("?? wip.txt");
  expect(siblingStatusBefore).toContain("?? wip.txt");
});

test("getGitLog with all:false returns only commits reachable from HEAD's current branch", async () => {
  const { getGitLog } = await import("./git.js");
  const repoDir = mkRepo();
  // main: A → B
  commitFile(repoDir, "a.txt", "a\n", "A on main");
  commitFile(repoDir, "b.txt", "b\n", "B on main");
  // feature off main with extra commit
  execFileSync("git", ["-C", repoDir, "checkout", "-b", "feature"], { stdio: "ignore" });
  commitFile(repoDir, "f.txt", "f\n", "F on feature");
  // back to main, add another
  execFileSync("git", ["-C", repoDir, "checkout", "main"], { stdio: "ignore" });
  commitFile(repoDir, "c.txt", "c\n", "C on main");

  const allLog = getGitLog(repoDir, { all: true });
  const allMessages = allLog.commits.map((c) => c.commit.message);
  expect(allMessages).toContain("F on feature");

  const currentLog = getGitLog(repoDir, { all: false });
  const currentMessages = currentLog.commits.map((c) => c.commit.message);
  expect(currentMessages).toContain("C on main");
  expect(currentMessages).toContain("B on main");
  expect(currentMessages).toContain("A on main");
  expect(currentMessages).not.toContain("F on feature");
});

test("listExistingWorktrees enumerates the main worktree plus every linked sibling", () => {
  const repoDir = mkRepo();
  const siblingA = mkdtempSync(join(tmpdir(), "oxplow-wt-a-"));
  tempDirs.push(siblingA);
  rmSync(siblingA, { recursive: true, force: true });
  execFileSync("git", ["-C", repoDir, "worktree", "add", "-b", "feat-a", siblingA], { stdio: "ignore" });

  const siblingB = mkdtempSync(join(tmpdir(), "oxplow-wt-b-"));
  tempDirs.push(siblingB);
  rmSync(siblingB, { recursive: true, force: true });
  execFileSync("git", ["-C", repoDir, "worktree", "add", "-b", "feat-b", siblingB], { stdio: "ignore" });

  const entries = listExistingWorktrees(repoDir);
  const byBranch = new Map(entries.map((wt) => [wt.branch, wt]));
  expect(byBranch.get("main")?.isMain).toBe(true);
  expect(byBranch.get("feat-a")?.isMain).toBe(false);
  expect(byBranch.get("feat-b")?.isMain).toBe(false);
  expect(entries.length).toBe(3);

  // Filtering by path (the runtime's listSiblingWorktrees logic) leaves the others.
  const siblings = entries.filter((wt) => wt.path !== entries.find((e) => e.isMain)!.path);
  expect(siblings.map((wt) => wt.branch).sort()).toEqual(["feat-a", "feat-b"]);
});

function commitFile(
  repoDir: string,
  name: string,
  contents: string,
  message: string,
  options?: { date?: string },
): void {
  commitFileIn(repoDir, name, contents, message, options);
}

function commitFileIn(
  dir: string,
  name: string,
  contents: string,
  message: string,
  options?: { date?: string },
): void {
  writeFileSync(join(dir, name), contents, "utf8");
  execFileSync("git", ["-C", dir, "add", name], { stdio: "ignore" });
  const env = options?.date
    ? { ...process.env, GIT_AUTHOR_DATE: options.date, GIT_COMMITTER_DATE: options.date }
    : process.env;
  execFileSync("git", ["-C", dir, "commit", "-m", message], { stdio: "ignore", env });
}

function mkBareRemote(): string {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-remote-"));
  tempDirs.push(dir);
  execFileSync("git", ["init", "--bare", "-b", "main", dir], { stdio: "ignore" });
  return dir;
}

function mkRepo(): string {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-git-test-"));
  tempDirs.push(dir);
  execFileSync("git", ["init", "-b", "main", dir], { stdio: "ignore" });
  execFileSync("git", ["-C", dir, "config", "user.name", "Test User"], { stdio: "ignore" });
  execFileSync("git", ["-C", dir, "config", "user.email", "test@example.com"], { stdio: "ignore" });
  writeFileSync(join(dir, "README.md"), "# test\n", "utf8");
  execFileSync("git", ["-C", dir, "add", "README.md"], { stdio: "ignore" });
  execFileSync("git", ["-C", dir, "commit", "-m", "init"], { stdio: "ignore" });
  return dir;
}
