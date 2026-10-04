// What's left of the legacy IPC types, until each moves to the generated
// bindings or beside its one consumer (P11: C8 the snake-case shapes, C9
// the git-ref types, then this file goes).

// ---- Branches & git ----

export interface BranchRef {
  kind: "local" | "remote";
  name: string;
  ref: string;
  remote?: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}

export type GitFileStatus = "modified" | "added" | "deleted" | "renamed" | "untracked";

export interface BranchChangeEntry {
  path: string;
  status: GitFileStatus;
  /** Optional line counts; unset on staged/unstaged where we don't compute them. */
  additions?: number | null;
  deletions?: number | null;
}

export interface ChangeScopes {
  /// Legacy "what's staged / unstaged / upstream / branchBase" arrays
  /// — empty under the new schema; the renderer uses `branchBase` /
  /// `upstream` / `currentBranch` strings via the new bindings now.
  staged: BranchChangeEntry[];
  unstaged: BranchChangeEntry[];
  upstream?: string;
  branchBase?: string;
  currentBranch?: string;
  onDefaultBranch?: boolean;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}

/// `GitLogResult` is a renderer-side aggregate that wraps the
/// bindings `GitLogCommit[]` with optional `currentBranch` /
/// `branchHeads` / `tags` overlay slots populated separately by
/// the renderer (e.g. via `listAllRefs`). The bindings type only
/// carries `commits`, so this stays in api-types until the Rust
/// surface grows the overlay or every consumer composes it
/// renderer-side.
export interface GroupedGitRefs {
  local: BranchRef[];
  remote: BranchRef[];
  /// Per-remote grouping the picker renders. Built from `remote` for
  /// renderer convenience.
  remotes?: { remote: string; branches: BranchRef[] }[];
  tags: { name: string; ref: string }[];
  /// Names (not refs) of the most-recently-checked-out local branches.
  /// Sorted recency-first.
  recent?: string[];
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}

export interface RefOption {
  ref: string;
  label: string;
  kind: "local" | "remote" | "tag";
  name?: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}

export interface TextSearchHit {
  path: string;
  line: number;
  preview: string;
  snippet?: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}

export interface BackgroundTask {
  id: string;
  kind: string;
  label: string;
  status: string;
  progress: number | null;
  startedAt: number;
  endedAt: number | null;
  error: string | null;
  result?: unknown;
  detail?: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}
