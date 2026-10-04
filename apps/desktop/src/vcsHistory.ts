/**
 * History, branches and tags read through the models (P5.B5,
 * `.context/vcs.md`): `v_commit`, `v_branch`, `v_tag`. The commit indexer
 * keeps them from every stream's head, so a stream's history is a
 * recursive walk over `parents` from its head — no VCS call. Each read
 * returns what it read (`reads`) so a caller re-runs with
 * `useRerunOnChange` when a model changes.
 */
import { querySql, type SqlCell } from "./api.js";
import type { Reads, RevisionInfo } from "./tauri-bridge/generated/bindings.js";

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

/** One ref a compare picker offers ([`readRefOptions`]). */
export interface RefOption {
  ref: string;
  label: string;
  kind: "local" | "remote" | "tag";
  name?: string;
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

export interface History {
  commits: RevisionInfo[];
  /** Local branch heads, for badges. */
  branchHeads: { name: string; sha: string }[];
  tags: TagRow[];
  reads: Reads;
}

const COMMIT_COLUMNS = "c.sha, c.author, c.email, c.committed_at, c.subject, c.parents";

/** A `v_commit` row (in `COMMIT_COLUMNS` order) as a `RevisionInfo`. */
export function commitFromRow(row: SqlCell[]): RevisionInfo {
  const [sha, author, email, committedAt, subject, parents] = row;
  const id = String(sha);
  let parentList: string[] = [];
  try {
    const parsed: unknown = JSON.parse(String(parents ?? "[]"));
    if (Array.isArray(parsed)) parentList = parsed.map(String);
  } catch {
    parentList = [];
  }
  return {
    id,
    short_id: id.slice(0, 7),
    author: String(author ?? ""),
    email: String(email ?? ""),
    time: Math.floor(Date.parse(String(committedAt)) / 1000),
    subject: String(subject ?? ""),
    parents: parentList,
  };
}

/** `commits` with every child before its parents (what the graph's lanes
 *  need), otherwise newest first in the order given. Commit times are
 *  whole seconds and clocks skew, so a time order alone can list a parent
 *  (a rebase's, a scripted series') before its child. */
export function topoOrder(commits: RevisionInfo[]): RevisionInfo[] {
  const present = new Set(commits.map((c) => c.id));
  const children = new Map<string, number>();
  for (const c of commits) {
    for (const p of c.parents) {
      if (present.has(p)) children.set(p, (children.get(p) ?? 0) + 1);
    }
  }
  const byId = new Map(commits.map((c) => [c.id, c]));
  const ready = commits.filter((c) => !children.get(c.id));
  const out: RevisionInfo[] = [];
  const rank = new Map(commits.map((c, i) => [c.id, i]));
  while (ready.length > 0) {
    // The newest ready commit; ties keep the order given.
    let best = 0;
    for (let i = 1; i < ready.length; i++) {
      const a = ready[i], b = ready[best];
      if (a.time > b.time || (a.time === b.time && rank.get(a.id)! < rank.get(b.id)!)) best = i;
    }
    const [next] = ready.splice(best, 1);
    out.push(next);
    for (const p of next.parents) {
      const left = (children.get(p) ?? 0) - 1;
      children.set(p, left);
      if (left === 0 && byId.has(p)) ready.push(byId.get(p)!);
    }
  }
  return out;
}

/** The SQL for a history read: from `head` along parents, or — `head`
 *  null — every indexed commit. Newest first, `?2` rows at most; the
 *  caller puts them in [`topoOrder`]. */
export function historySql(head: string | null): string {
  if (head === null) {
    return `SELECT ${COMMIT_COLUMNS} FROM v_commit c
            ORDER BY c.committed_at DESC, c.sha LIMIT ?1`;
  }
  return `WITH RECURSIVE reach(sha) AS (
            SELECT ?1
            UNION
            SELECT p.value FROM reach
              JOIN v_commit c ON c.sha = reach.sha, json_each(c.parents) p
          )
          SELECT ${COMMIT_COLUMNS} FROM v_commit c JOIN reach USING (sha)
          ORDER BY c.committed_at DESC, c.sha LIMIT ?2`;
}

/** A stream's history from its head commit (`head`), or every branch's
 *  (`head` null), with branch-head and tag badges. */
export async function readHistory(head: string | null, limit: number): Promise<History> {
  const [commits, branches, tags] = await Promise.all([
    querySql(historySql(head), head === null ? [limit] : [head, limit], limit),
    querySql(
      "SELECT name, head_sha FROM v_branch WHERE kind = 'local' AND head_sha IS NOT NULL ORDER BY name",
    ),
    querySql("SELECT name, sha FROM v_tag ORDER BY name"),
  ]);
  return {
    commits: topoOrder(commits.rows.map(commitFromRow)),
    branchHeads: branches.rows.map(([name, sha]) => ({ name: String(name), sha: String(sha) })),
    tags: tags.rows.map(([name, sha]) => ({ name: String(name), sha: String(sha) })),
    reads: {
      models: [...new Set([...commits.reads.models, ...branches.reads.models, ...tags.reads.models])],
      tables: [],
      measures: [],
    },
  };
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

/** Every branch and tag as one flat list — what "Compare with…" offers. */
export async function readRefOptions(): Promise<RefOption[]> {
  const groups = await readRefGroups();
  return [
    ...groups.local.map((b) => ({ ref: b.ref, label: b.name, kind: "local" as const, name: b.name })),
    ...groups.remote.map((b) => ({
      ref: b.ref,
      label: `${b.remote}/${b.name}`,
      kind: "remote" as const,
      name: `${b.remote}/${b.name}`,
    })),
    ...groups.tags.map((t) => ({ ref: t.ref, label: t.name, kind: "tag" as const, name: t.name })),
  ];
}
