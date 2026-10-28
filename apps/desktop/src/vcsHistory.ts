/**
 * History, branches and tags read through the datasets (P5.B5,
 * `.context/vcs.md`): `v_commit`, `v_branch`, `v_tag`. The commit indexer
 * keeps them from every stream's head, so a stream's history is a
 * recursive walk over `parents` from its head — no VCS call. Each read
 * returns what it read (`reads`) so a caller re-runs with
 * `useRerunOnChange` when a dataset changes.
 */
import { querySql } from "./api.js";
import type { Reads } from "./tauri-bridge/generated/bindings.js";

/** A branch as the pickers show it: a git ref name (`refs/heads/main`,
 *  `refs/remotes/origin/main`) and, for a remote one, its remote. */
export interface BranchRef {
  kind: "local" | "remote";
  name: string;
  ref: string;
  remote?: string;
}

/** The refs grouped as the branch picker shows them ([`readRefGroups`]). */
export interface GroupedGitRefs {
  local: BranchRef[];
  remote: BranchRef[];
  /** Per remote, its branches. */
  remotes?: { remote: string; branches: BranchRef[] }[];
  tags: { name: string; ref: string }[];
  /** Names of the most recently checked-out local branches, latest first. */
  recent?: string[];
}

/** A branch as `v_branch` holds it. */
export interface BranchRow {
  name: string;
  /** The remote it tracks a copy of; null for a local branch. */
  remote: string | null;
  /** The commit it points at. */
  head: string | null;
  isDefault: boolean;
  /** The stream whose worktree has it checked out, if any. */
  streamId: number | null;
}

/** A tag as `v_tag` holds it. */
export interface TagRow {
  name: string;
  sha: string;
}

/** Every branch, local first then remote-tracking, each by name. */
export async function readBranches(): Promise<{ branches: BranchRow[]; reads: Reads }> {
  const res = await querySql(
    `SELECT name, remote, head_sha, is_default, stream_id FROM v_branch
      ORDER BY kind = 'remote', remote, name`,
  );
  return {
    branches: res.rows.map(([name, remote, head, isDefault, streamId]) => ({
      name: String(name),
      remote: remote === null ? null : String(remote),
      head: head === null ? null : String(head),
      isDefault: Number(isDefault) === 1,
      streamId: streamId === null ? null : Number(streamId),
    })),
    reads: res.reads,
  };
}

/** Every tag, by name. */
export async function readTags(): Promise<{ tags: TagRow[]; reads: Reads }> {
  const res = await querySql("SELECT name, sha FROM v_tag ORDER BY name");
  return {
    tags: res.rows.map(([name, sha]) => ({ name: String(name), sha: String(sha) })),
    reads: res.reads,
  };
}

/** A branch row as the pickers' `BranchRef` (a git ref name: branch
 *  switching and stream creation still take git refs). */
export function branchRefOf(row: BranchRow): BranchRef {
  if (row.remote === null) {
    return { kind: "local", name: row.name, ref: `refs/heads/${row.name}` };
  }
  return {
    kind: "remote",
    name: row.name,
    ref: `refs/remotes/${row.remote}/${row.name}`,
    remote: row.remote,
  };
}

/** Branches and tags grouped the way the branch picker shows them. */
export async function readRefGroups(): Promise<GroupedGitRefs> {
  const [{ branches }, { tags }] = await Promise.all([readBranches(), readTags()]);
  const local = branches.filter((b) => b.remote === null).map(branchRefOf);
  const remote = branches.filter((b) => b.remote !== null).map(branchRefOf);
  const byRemote = new Map<string, BranchRef[]>();
  for (const b of remote) byRemote.set(b.remote!, [...(byRemote.get(b.remote!) ?? []), b]);
  return {
    local,
    remote,
    remotes: [...byRemote.entries()].map(([name, list]) => ({ remote: name, branches: list })),
    tags: tags.map((t) => ({ name: t.name, ref: `refs/tags/${t.name}` })),
    recent: local.slice(0, 5).map((b) => b.name),
  };
}
