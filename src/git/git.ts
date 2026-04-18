import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, realpathSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
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

/**
 * True if `projectDir` is a secondary git worktree (its `.git` is a file
 * pointing at the main repo's worktrees/ dir, not a regular `.git` directory).
 * We refuse to start newde in a worktree because newde manages its own
 * worktrees under `.newde/worktrees/` and nesting one inside a user-created
 * worktree makes pane/stream accounting incoherent.
 */
export function isGitWorktree(projectDir: string): boolean {
  try {
    const dotGit = join(projectDir, ".git");
    const stats = statSync(dotGit);
    return stats.isFile();
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

export interface BranchChangeEntry {
  path: string;
  status: GitFileStatus;
  additions: number | null;
  deletions: number | null;
}

export interface BranchChanges {
  baseRef: string;
  mergeBase: string | null;
  files: BranchChangeEntry[];
}

/**
 * Best-effort base-branch heuristic: prefer remote refs over local so that
 * "changed vs origin" is the default. Falls back through `origin/main` →
 * `main` → `origin/master` → `master` → `origin/HEAD`. Returns null when
 * none of those exist.
 */
export function detectBaseBranch(projectDir: string): string | null {
  if (!isGitRepo(projectDir)) return null;
  const candidates = ["origin/main", "main", "origin/master", "master"];
  for (const candidate of candidates) {
    if (refExists(projectDir, candidate)) return candidate;
  }
  try {
    const out = execFileSync("git", ["-C", projectDir, "symbolic-ref", "--short", "refs/remotes/origin/HEAD"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
    if (out) return out;
  } catch {}
  return null;
}

/**
 * Returns the list of files that differ between the current working tree
 * (including uncommitted changes) and the merge base with `baseRef`. Mirrors
 * IntelliJ's "changes vs branch" view — everything this branch would add in a
 * PR, plus wip edits on top.
 */
export function listBranchChanges(projectDir: string, baseRef: string): BranchChanges {
  if (!isGitRepo(projectDir)) {
    return { baseRef, mergeBase: null, files: [] };
  }
  let mergeBase: string | null = null;
  try {
    mergeBase = execFileSync("git", ["-C", projectDir, "merge-base", "HEAD", baseRef], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim() || null;
  } catch {
    return { baseRef, mergeBase: null, files: [] };
  }
  if (!mergeBase) return { baseRef, mergeBase: null, files: [] };

  const diffOut = safeGit(projectDir, ["diff", "--name-status", "-z", mergeBase]);
  const numstatOut = safeGit(projectDir, ["diff", "--numstat", "-z", mergeBase]);
  const nameStatus = parseNameStatusZ(diffOut);
  const numstat = parseNumstatZ(numstatOut);
  // Include untracked files via status --porcelain, since `diff` doesn't list them.
  const untracked = listUntrackedFiles(projectDir);

  const byPath = new Map<string, BranchChangeEntry>();
  for (const entry of nameStatus) {
    const stats = numstat.get(entry.path);
    byPath.set(entry.path, {
      path: entry.path,
      status: entry.status,
      additions: stats?.additions ?? null,
      deletions: stats?.deletions ?? null,
    });
  }
  for (const path of untracked) {
    if (!byPath.has(path)) {
      byPath.set(path, { path, status: "untracked", additions: null, deletions: null });
    }
  }
  return {
    baseRef,
    mergeBase,
    files: [...byPath.values()].sort((a, b) => a.path.localeCompare(b.path)),
  };
}

/**
 * Returns file content at a specific ref (commit, tag, branch, merge base).
 * `null` when the file doesn't exist at that ref (e.g. brand-new file).
 */
export function readFileAtRef(projectDir: string, ref: string, path: string): string | null {
  if (!isGitRepo(projectDir)) return null;
  try {
    return execFileSync("git", ["-C", projectDir, "show", `${ref}:${path}`], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
      maxBuffer: 64 * 1024 * 1024,
    });
  } catch {
    return null;
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

function refExists(projectDir: string, ref: string): boolean {
  try {
    execFileSync("git", ["-C", projectDir, "rev-parse", "--verify", "--quiet", ref], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function safeGit(projectDir: string, args: string[]): string {
  try {
    return execFileSync("git", ["-C", projectDir, ...args], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
      maxBuffer: 64 * 1024 * 1024,
    });
  } catch {
    return "";
  }
}

// `git diff --name-status -z` emits <status>\0<path>\0 for additions/deletions/
// modifications, and <status>\0<old>\0<new>\0 for renames/copies (R/C). The
// status prefix can be "R100", "C75", etc. — we care about the leading letter.
function parseNameStatusZ(out: string): Array<{ status: GitFileStatus; path: string }> {
  if (!out) return [];
  const tokens = out.split("\0").filter((t) => t.length > 0);
  const result: Array<{ status: GitFileStatus; path: string }> = [];
  for (let i = 0; i < tokens.length; i++) {
    const code = tokens[i]!;
    const head = code[0]!;
    if (head === "R" || head === "C") {
      // old path is tokens[i+1], new path is tokens[i+2]
      const newPath = tokens[i + 2];
      if (newPath) result.push({ status: "renamed", path: newPath });
      i += 2;
      continue;
    }
    const path = tokens[i + 1];
    if (!path) continue;
    result.push({ status: nameStatusToGitFileStatus(head), path });
    i += 1;
  }
  return result;
}

function nameStatusToGitFileStatus(code: string): GitFileStatus {
  if (code === "A") return "added";
  if (code === "D") return "deleted";
  if (code === "R") return "renamed";
  return "modified";
}

function parseNumstatZ(out: string): Map<string, { additions: number; deletions: number }> {
  const map = new Map<string, { additions: number; deletions: number }>();
  if (!out) return map;
  const tokens = out.split("\0").filter((t) => t.length > 0);
  // Each record: "<add>\t<del>\t<path>" as a single token (numstat -z only
  // splits rename records across two extra tokens).
  for (let i = 0; i < tokens.length; i++) {
    const token = tokens[i]!;
    const [addStr, delStr, maybePath] = token.split("\t");
    if (addStr == null || delStr == null) continue;
    const additions = addStr === "-" ? 0 : Number(addStr);
    const deletions = delStr === "-" ? 0 : Number(delStr);
    if (maybePath) {
      map.set(maybePath, { additions, deletions });
    } else {
      // Rename: path fields follow as two more tokens (old, new); numstat
      // attributes the counts to the new path.
      const newPath = tokens[i + 2];
      if (newPath) map.set(newPath, { additions, deletions });
      i += 2;
    }
  }
  return map;
}

function listUntrackedFiles(projectDir: string): string[] {
  const out = safeGit(projectDir, ["ls-files", "--others", "--exclude-standard", "-z"]);
  if (!out) return [];
  return out.split("\0").filter((line) => line.length > 0);
}
