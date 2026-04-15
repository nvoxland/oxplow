import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { resolve, sep } from "node:path";

export type GitFileStatus = "modified" | "added" | "deleted" | "renamed" | "untracked";

export interface WorkspaceEntry {
  name: string;
  path: string;
  kind: "file" | "directory";
  gitStatus: GitFileStatus | null;
  hasChanges: boolean;
}

export interface WorkspaceFile {
  path: string;
  content: string;
}

export interface WorkspaceIndexedFile {
  path: string;
  gitStatus: GitFileStatus | null;
}

export interface WorkspaceStatusSummary {
  modified: number;
  added: number;
  deleted: number;
  renamed: number;
  untracked: number;
  total: number;
}

export function listWorkspaceEntries(
  rootDir: string,
  relativePath: string,
  gitStatuses: Map<string, GitFileStatus> = new Map(),
): WorkspaceEntry[] {
  const dir = resolveWorkspacePath(rootDir, relativePath);
  const entries = readdirSync(dir, { withFileTypes: true })
    .map((entry) => {
      const path = normalizeRelativePath(relativePath, entry.name);
      const kind = entry.isDirectory() ? "directory" as const : "file" as const;
      const gitStatus = kind === "file" ? (gitStatuses.get(path) ?? null) : null;
      const hasChanges = kind === "directory"
        ? hasDescendantChanges(path, gitStatuses)
        : gitStatus !== null;
      return {
        name: entry.name,
        path,
        kind,
        gitStatus,
        hasChanges,
      };
    });

  entries.sort((a, b) => {
    if (a.kind !== b.kind) return a.kind === "directory" ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
  return entries;
}

export function listWorkspaceFiles(
  rootDir: string,
  gitStatuses: Map<string, GitFileStatus> = new Map(),
  relativePath = "",
): WorkspaceIndexedFile[] {
  const dir = resolveWorkspacePath(rootDir, relativePath);
  const files: WorkspaceIndexedFile[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = normalizeRelativePath(relativePath, entry.name);
    if (entry.isDirectory()) {
      files.push(...listWorkspaceFiles(rootDir, gitStatuses, path));
    } else {
      files.push({
        path,
        gitStatus: gitStatuses.get(path) ?? null,
      });
    }
  }
  files.sort((a, b) => a.path.localeCompare(b.path));
  return files;
}

export function readWorkspaceFile(rootDir: string, relativePath: string): WorkspaceFile {
  const path = cleanRelativePath(relativePath);
  const abs = resolveWorkspacePath(rootDir, path);
  return {
    path,
    content: readFileSync(abs, "utf8"),
  };
}

export function writeWorkspaceFile(rootDir: string, relativePath: string, content: string): WorkspaceFile {
  const path = cleanRelativePath(relativePath);
  const abs = resolveWorkspacePath(rootDir, path);
  writeFileSync(abs, content, "utf8");
  return { path, content };
}

export function summarizeGitStatuses(gitStatuses: Map<string, GitFileStatus>): WorkspaceStatusSummary {
  const summary: WorkspaceStatusSummary = {
    modified: 0,
    added: 0,
    deleted: 0,
    renamed: 0,
    untracked: 0,
    total: 0,
  };
  for (const status of gitStatuses.values()) {
    summary[status] += 1;
    summary.total += 1;
  }
  return summary;
}

function hasDescendantChanges(path: string, gitStatuses: Map<string, GitFileStatus>): boolean {
  const prefix = path + "/";
  for (const changedPath of gitStatuses.keys()) {
    if (changedPath === path || changedPath.startsWith(prefix)) return true;
  }
  return false;
}

function normalizeRelativePath(base: string, name: string): string {
  return base ? `${base}/${name}` : name;
}

function cleanRelativePath(relativePath: string): string {
  return relativePath.replace(/^\/+/, "");
}

function resolveWorkspacePath(rootDir: string, relativePath: string): string {
  const clean = cleanRelativePath(relativePath);
  const root = resolve(rootDir);
  const abs = resolve(rootDir, clean);
  if (abs !== root && !abs.startsWith(root + sep)) {
    throw new Error("path resolves outside workspace");
  }
  return abs;
}
