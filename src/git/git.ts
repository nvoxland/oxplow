import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, realpathSync } from "node:fs";
import { dirname, resolve } from "node:path";
import type { GitFileStatus } from "./workspace-files.js";

export interface BranchRef {
  kind: "local" | "remote";
  name: string;
  ref: string;
  remote?: string;
}

export function detectCurrentBranch(projectDir: string): string | null {
  if (!isGitRepo(projectDir)) return null;
  try {
    const out = execFileSync("git", ["-C", projectDir, "rev-parse", "--abbrev-ref", "HEAD"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
    if (!out || out === "HEAD") return null;
    return out;
  } catch {
    return null;
  }
}

export function isGitRepo(projectDir: string): boolean {
  try {
    const root = execFileSync("git", ["-C", projectDir, "rev-parse", "--show-toplevel"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
    return !!root && canonicalPath(root) === canonicalPath(projectDir);
  } catch {
    return false;
  }
}

export function listBranches(projectDir: string): BranchRef[] {
  if (!isGitRepo(projectDir)) return [];
  const local = gitLines(projectDir, [
    "for-each-ref",
    "--format=%(refname)|%(refname:short)",
    "refs/heads",
  ])
    .map((line) => parseBranchLine(line, "local"))
    .filter((v): v is BranchRef => v !== null);
  const remote = gitLines(projectDir, [
    "for-each-ref",
    "--format=%(refname)|%(refname:short)",
    "refs/remotes",
  ])
    .map((line) => parseBranchLine(line, "remote"))
    .filter((v): v is BranchRef => v !== null)
    .filter((branch) => !branch.name.endsWith("/HEAD"));

  return [...local, ...remote].sort((a, b) => {
    if (a.kind !== b.kind) return a.kind.localeCompare(b.kind);
    return a.name.localeCompare(b.name);
  });
}

export function branchExists(projectDir: string, branch: string): boolean {
  if (!isGitRepo(projectDir)) return false;
  try {
    execFileSync("git", ["-C", projectDir, "show-ref", "--verify", "--quiet", `refs/heads/${branch}`], {
      stdio: "ignore",
    });
    return true;
  } catch {
    return false;
  }
}

export function ensureWorktree(
  projectDir: string,
  worktreePath: string,
  input:
    | { kind: "existing-local"; branch: string }
    | { kind: "existing-remote"; branch: string; remoteRef: string }
    | { kind: "new"; branch: string; startPoint: string },
): void {
  if (!isGitRepo(projectDir)) {
    throw new Error(`cannot create worktree outside a git repo root: ${projectDir}`);
  }
  if (isGitRepo(worktreePath)) return;
  if (existsSync(worktreePath)) {
    throw new Error(`worktree path already exists and is not a git repo: ${worktreePath}`);
  }
  mkdirSync(dirname(worktreePath), { recursive: true });

  switch (input.kind) {
    case "existing-local":
      git(projectDir, ["worktree", "add", worktreePath, input.branch]);
      return;
    case "existing-remote":
      if (branchExists(projectDir, input.branch)) {
        git(projectDir, ["worktree", "add", worktreePath, input.branch]);
        git(worktreePath, ["branch", "--set-upstream-to", input.remoteRef, input.branch]);
        return;
      }
      git(projectDir, ["worktree", "add", "-b", input.branch, worktreePath, input.remoteRef]);
      git(worktreePath, ["branch", "--set-upstream-to", input.remoteRef, input.branch]);
      return;
    case "new":
      git(projectDir, ["worktree", "add", "-b", input.branch, worktreePath, input.startPoint]);
      return;
  }
}

export function listGitStatuses(projectDir: string): Map<string, GitFileStatus> {
  const statuses = new Map<string, GitFileStatus>();
  if (!isGitRepo(projectDir)) return statuses;
  const out = git(projectDir, ["status", "--porcelain"]);
  if (!out) return statuses;

  for (const line of out.split("\n")) {
    if (!line) continue;
    const code = line.slice(0, 2);
    const rawPath = line.slice(3).trim();
    const path = rawPath.includes(" -> ") ? rawPath.split(" -> ").at(-1)! : rawPath;
    statuses.set(path, parseGitFileStatus(code));
  }

  return statuses;
}

function parseBranchLine(line: string, kind: "local" | "remote"): BranchRef | null {
  if (!line) return null;
  const [ref, shortName] = line.split("|");
  if (!ref || !shortName) return null;
  if (kind === "local") {
    return { kind, name: shortName, ref };
  }
  const slash = shortName.indexOf("/");
  return {
    kind,
    name: shortName,
    ref,
    remote: slash > 0 ? shortName.slice(0, slash) : undefined,
  };
}

function git(projectDir: string, args: string[]): string {
  return execFileSync("git", ["-C", projectDir, ...args], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  }).trimEnd();
}

function gitLines(projectDir: string, args: string[]): string[] {
  const out = git(projectDir, args);
  if (!out) return [];
  return out.split("\n").filter((line) => line.length > 0);
}

function parseGitFileStatus(code: string): GitFileStatus {
  if (code.includes("?")) return "untracked";
  if (code.includes("R")) return "renamed";
  if (code.includes("A")) return "added";
  if (code.includes("D")) return "deleted";
  return "modified";
}

function canonicalPath(path: string): string {
  return realpathSync(path);
}
