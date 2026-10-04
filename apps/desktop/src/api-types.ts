// What's left of the legacy IPC types: the git-ref shapes, until they move
// beside their one consumer (P11, C9) and this file goes.

// ---- Branches & git ----

export interface BranchRef {
  kind: "local" | "remote";
  name: string;
  ref: string;
  remote?: string;
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
