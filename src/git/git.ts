import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
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
/**
 * Returns the upstream ref (e.g. `origin/feature-x`) for the current branch,
 * or `null` when no upstream is configured. Used to compute "unpushed" changes
 * as the working tree + any commits on HEAD that aren't in `@{u}`.
 */
export function detectUpstreamRef(projectDir: string): string | null {
  if (!isGitRepo(projectDir)) return null;
  try {
    const out = execFileSync(
      "git",
      ["-C", projectDir, "rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
      { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] },
    ).trim();
    return out || null;
  } catch {
    return null;
  }
}

export interface ChangeScopes {
  currentBranch: string | null;
  branchBase: string | null;
  upstream: string | null;
  onDefaultBranch: boolean;
}

export function getChangeScopes(projectDir: string): ChangeScopes {
  const currentBranch = detectCurrentBranch(projectDir);
  const branchBase = detectBaseBranch(projectDir);
  const upstream = detectUpstreamRef(projectDir);
  const baseName = branchBase?.replace(/^origin\//, "") ?? null;
  const onDefaultBranch = !!currentBranch && baseName === currentBranch;
  return { currentBranch, branchBase, upstream, onDefaultBranch };
}

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
export interface GitLogCommit {
  sha: string;
  parents: Array<{ sha: string }>;
  commit: {
    author: { name: string; email: string; date: string };
    message: string;
  };
  refs: string[];
}

export interface GitLogRef {
  name: string;
  commit: { sha: string };
}

export interface GitLogResult {
  commits: GitLogCommit[];
  branchHeads: GitLogRef[];
  tags: GitLogRef[];
  currentBranch: string | null;
}

const UNIT = "\x1f";
const RECORD = "\x1e";

export function getGitLog(projectDir: string, options?: { limit?: number }): GitLogResult {
  const limit = Math.max(1, Math.min(options?.limit ?? 500, 5000));
  const format = ["%H", "%P", "%an", "%ae", "%aI", "%s", "%D"].join(UNIT) + RECORD;
  let raw: string;
  try {
    raw = git(projectDir, ["log", "--all", "--date-order", `--max-count=${limit}`, `--pretty=format:${format}`]);
  } catch {
    return { commits: [], branchHeads: [], tags: [], currentBranch: detectCurrentBranch(projectDir) };
  }
  const branchHeads = new Map<string, string>();
  const tags = new Map<string, string>();
  const commits: GitLogCommit[] = [];
  for (const record of raw.split(RECORD)) {
    const line = record.replace(/^\n/, "");
    if (!line) continue;
    const [sha, parentsRaw, authorName, authorEmail, authorDate, subject, refsRaw] = line.split(UNIT);
    if (!sha) continue;
    const parents = (parentsRaw ?? "")
      .split(/\s+/)
      .filter(Boolean)
      .map((p) => ({ sha: p }));
    const refs: string[] = [];
    for (const ref of (refsRaw ?? "").split(",").map((r) => r.trim()).filter(Boolean)) {
      refs.push(ref);
      if (ref.startsWith("tag: ")) {
        const name = ref.slice(5);
        if (name) tags.set(name, sha);
        continue;
      }
      // HEAD -> branch, or plain branch name (local or remote)
      const name = ref.replace(/^HEAD -> /, "");
      if (name === "HEAD") continue;
      if (!branchHeads.has(name)) branchHeads.set(name, sha);
    }
    commits.push({
      sha,
      parents,
      commit: {
        author: { name: authorName ?? "", email: authorEmail ?? "", date: authorDate ?? "" },
        message: subject ?? "",
      },
      refs,
    });
  }
  return {
    commits,
    branchHeads: [...branchHeads.entries()].map(([name, sha]) => ({ name, commit: { sha } })),
    tags: [...tags.entries()].map(([name, sha]) => ({ name, commit: { sha } })),
    currentBranch: detectCurrentBranch(projectDir),
  };
}

export interface CommitDetail {
  sha: string;
  parents: string[];
  author: { name: string; email: string; date: string };
  committer: { name: string; email: string; date: string };
  subject: string;
  body: string;
  files: Array<{ path: string; status: GitFileStatus; additions: number; deletions: number }>;
}

export function getCommitDetail(projectDir: string, sha: string): CommitDetail | null {
  // Header: sha \x1f parents \x1f author \x1f author_email \x1f author_date
  //   \x1f committer \x1f committer_email \x1f committer_date \x1f subject \x1e body
  const format = ["%H", "%P", "%an", "%ae", "%aI", "%cn", "%ce", "%cI", "%s"].join(UNIT) + RECORD + "%b";
  let raw: string;
  try {
    raw = git(projectDir, ["show", "--format=" + format, "--name-status", "--numstat", "-z", sha]);
  } catch {
    return null;
  }
  // `git show -z` terminates name-status/numstat entries with NUL. The header
  // (with our custom %x1f/%x1e separators) comes first, then a blank line, then
  // name-status entries (NUL-separated), then numstat (NUL-separated), all in
  // one blob. We parse the header first, then split the rest on NUL.
  const recordIdx = raw.indexOf(RECORD);
  if (recordIdx < 0) return null;
  const headerLine = raw.slice(0, recordIdx);
  const rest = raw.slice(recordIdx + 1);
  const headerParts = headerLine.split(UNIT);
  const [sha2, parentsRaw, aName, aEmail, aDate, cName, cEmail, cDate, subject] = headerParts;
  if (!sha2) return null;

  // rest starts with body up to the first blank-line marker. With `-z`, git
  // emits the body then a NUL-terminated run of entries. The body ends at the
  // first NUL that isn't inside body text — git always appends a NUL after
  // the body when --name-status/--numstat follow.
  const firstNul = rest.indexOf("\0");
  const body = firstNul < 0 ? rest : rest.slice(0, firstNul);
  const tail = firstNul < 0 ? "" : rest.slice(firstNul + 1);

  const entries = tail.split("\0").filter((s) => s.length > 0);
  const files = new Map<string, { path: string; status: GitFileStatus; additions: number; deletions: number }>();
  let i = 0;
  // name-status entries: [status, path] or [status, from, to] for rename.
  while (i < entries.length) {
    const statusCode = entries[i]!;
    if (!/^[A-Z]/.test(statusCode)) break; // name-status done → numstat starts
    if (statusCode.startsWith("R") || statusCode.startsWith("C")) {
      const from = entries[i + 1];
      const to = entries[i + 2];
      if (from && to) {
        files.set(to, { path: `${from} → ${to}`, status: nameStatusToGitFileStatus(statusCode), additions: 0, deletions: 0 });
      }
      i += 3;
    } else {
      const path = entries[i + 1];
      if (path) {
        files.set(path, { path, status: nameStatusToGitFileStatus(statusCode), additions: 0, deletions: 0 });
      }
      i += 2;
    }
  }
  // numstat entries: [additions \t deletions \t path] (or with tabs) — actually
  // git emits them as one string with tabs; for renames the path is NUL-split
  // into two entries. Detect "<n>\t<n>\t<path>" vs "<n>\t<n>" followed by two paths.
  while (i < entries.length) {
    const entry = entries[i]!;
    const match = entry.match(/^(-|\d+)\t(-|\d+)(?:\t(.*))?$/);
    if (!match) { i++; continue; }
    const additions = match[1] === "-" ? 0 : Number(match[1]);
    const deletions = match[2] === "-" ? 0 : Number(match[2]);
    let path = match[3] ?? "";
    let consumed = 1;
    if (!path) {
      // rename: next two entries are from, to
      const from = entries[i + 1];
      const to = entries[i + 2];
      if (from && to) path = to;
      consumed = 3;
    }
    const existing = files.get(path);
    if (existing) { existing.additions = additions; existing.deletions = deletions; }
    i += consumed;
  }

  return {
    sha: sha2,
    parents: (parentsRaw ?? "").split(/\s+/).filter(Boolean),
    author: { name: aName ?? "", email: aEmail ?? "", date: aDate ?? "" },
    committer: { name: cName ?? "", email: cEmail ?? "", date: cDate ?? "" },
    subject: subject ?? "",
    body: body.replace(/^\n+|\n+$/g, ""),
    files: [...files.values()],
  };
}

export interface TextSearchHit {
  path: string;
  line: number;
  snippet: string;
}

/**
 * Runs a literal-text search across the working tree using `git grep`.
 * Good enough for a "find usages" starter — respects .gitignore, indexes
 * tracked+untracked, limits results and line length so a careless query
 * can't balloon the response.
 */
export function searchWorkspaceText(projectDir: string, query: string, options?: { limit?: number }): TextSearchHit[] {
  const trimmed = query.trim();
  if (!trimmed || !isGitRepo(projectDir)) return [];
  const limit = Math.max(1, Math.min(options?.limit ?? 200, 1000));
  let raw: string;
  try {
    raw = execFileSync(
      "git",
      ["-C", projectDir, "grep", "--no-color", "-n", "-I", "-F", "--untracked", "--", trimmed],
      { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], maxBuffer: 32 * 1024 * 1024 },
    );
  } catch {
    // git grep exits 1 on "no matches" — treat as empty.
    return [];
  }
  const out: TextSearchHit[] = [];
  for (const line of raw.split("\n")) {
    if (out.length >= limit) break;
    // Format: path:lineNumber:snippet
    const firstColon = line.indexOf(":");
    if (firstColon < 0) continue;
    const secondColon = line.indexOf(":", firstColon + 1);
    if (secondColon < 0) continue;
    const path = line.slice(0, firstColon);
    const lineNo = Number(line.slice(firstColon + 1, secondColon));
    if (!Number.isFinite(lineNo)) continue;
    let snippet = line.slice(secondColon + 1);
    if (snippet.length > 400) snippet = snippet.slice(0, 400) + "…";
    out.push({ path, line: lineNo, snippet });
  }
  return out;
}

export interface GitOpResult {
  ok: boolean;
  stdout: string;
  stderr: string;
  exitCode: number | null;
}

function runGit(projectDir: string, args: string[]): GitOpResult {
  try {
    const stdout = execFileSync("git", ["-C", projectDir, ...args], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    return { ok: true, stdout, stderr: "", exitCode: 0 };
  } catch (error) {
    const err = error as { status?: number; stdout?: string; stderr?: string; message?: string };
    return {
      ok: false,
      stdout: err.stdout ?? "",
      stderr: err.stderr ?? err.message ?? "unknown git error",
      exitCode: typeof err.status === "number" ? err.status : null,
    };
  }
}

export function restorePath(projectDir: string, path: string): GitOpResult {
  // `checkout HEAD -- <path>` restores both staged and working-tree copies,
  // matching IntelliJ's "Rollback" behaviour for a single file.
  return runGit(projectDir, ["checkout", "HEAD", "--", path]);
}

export function addPath(projectDir: string, path: string): GitOpResult {
  return runGit(projectDir, ["add", "--", path]);
}

/** Append a line to .gitignore at the repo root, creating the file if needed. */
export function appendToGitignore(projectDir: string, path: string): GitOpResult {
  const gitignorePath = join(projectDir, ".gitignore");
  const entry = path.startsWith("/") ? path : `/${path}`;
  let existing = "";
  if (existsSync(gitignorePath)) {
    try {
      existing = readFileSync(gitignorePath, "utf8");
    } catch (error) {
      return { ok: false, stdout: "", stderr: (error as Error).message, exitCode: null };
    }
  }
  const lines = existing.split("\n").map((line) => line.trim());
  if (lines.includes(entry.trim()) || lines.includes(path.trim())) {
    return { ok: true, stdout: `${path} already in .gitignore`, stderr: "", exitCode: 0 };
  }
  const next = existing.length > 0 && !existing.endsWith("\n")
    ? `${existing}\n${entry}\n`
    : `${existing}${entry}\n`;
  try {
    writeFileSync(gitignorePath, next);
    return { ok: true, stdout: `added ${entry}`, stderr: "", exitCode: 0 };
  } catch (error) {
    return { ok: false, stdout: "", stderr: (error as Error).message, exitCode: null };
  }
}

export function gitPush(projectDir: string, options?: { force?: boolean; setUpstream?: boolean; remote?: string; branch?: string }): GitOpResult {
  const args = ["push"];
  if (options?.force) args.push("--force-with-lease");
  if (options?.setUpstream) args.push("--set-upstream");
  if (options?.remote) args.push(options.remote);
  if (options?.branch) args.push(options.branch);
  return runGit(projectDir, args);
}

export function gitPull(projectDir: string, options?: { rebase?: boolean; remote?: string; branch?: string }): GitOpResult {
  const args = ["pull"];
  if (options?.rebase) args.push("--rebase");
  if (options?.remote) args.push(options.remote);
  if (options?.branch) args.push(options.branch);
  return runGit(projectDir, args);
}

/**
 * Commits that touched a given path, newest first. Uses --follow so we trace
 * through renames. Returns the same shape as getGitLog so UIs can reuse the
 * existing commit rendering.
 */
export function listFileCommits(projectDir: string, path: string, limit = 50): GitLogCommit[] {
  if (!isGitRepo(projectDir)) return [];
  const format = ["%H", "%P", "%an", "%ae", "%aI", "%s", "%D"].join(UNIT) + RECORD;
  let raw: string;
  try {
    raw = git(projectDir, ["log", "--follow", `--max-count=${Math.min(limit, 500)}`, `--pretty=format:${format}`, "--", path]);
  } catch {
    return [];
  }
  const commits: GitLogCommit[] = [];
  for (const record of raw.split(RECORD)) {
    const line = record.replace(/^\n/, "");
    if (!line) continue;
    const [sha, parentsRaw, authorName, authorEmail, authorDate, subject, refsRaw] = line.split(UNIT);
    if (!sha) continue;
    const parents = (parentsRaw ?? "").split(/\s+/).filter(Boolean).map((p) => ({ sha: p }));
    const refs = (refsRaw ?? "").split(",").map((r) => r.trim()).filter(Boolean);
    commits.push({
      sha,
      parents,
      commit: {
        author: { name: authorName ?? "", email: authorEmail ?? "", date: authorDate ?? "" },
        message: subject ?? "",
      },
      refs,
    });
  }
  return commits;
}

export interface RefOption {
  kind: "branch" | "tag";
  name: string;
  ref: string;
}

export function listAllRefs(projectDir: string): RefOption[] {
  if (!isGitRepo(projectDir)) return [];
  const out: RefOption[] = [];
  for (const branch of listBranches(projectDir)) {
    out.push({ kind: "branch", name: branch.name, ref: branch.ref });
  }
  try {
    const tags = gitLines(projectDir, ["tag", "--list"]);
    for (const tag of tags) out.push({ kind: "tag", name: tag, ref: tag });
  } catch {}
  return out;
}

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
