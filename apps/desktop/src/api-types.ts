// Legacy Electron IPC contract — kept here to keep the existing UI
// typechecking through the migration. The runtime side is dead;
// `window.oxplowApi` doesn't exist under Tauri. Calls into
// `desktopApi().*` will throw "not yet ported" until each method is
// wired through the new `tauri-bridge`.
//
// New UI code should import from `./tauri-bridge/index.ts` directly,
// which gives a typed surface backed by real Tauri commands.

// (No bridge imports — the api-types module is self-contained for typecheck.)

// ---- Stream / Thread / Task (kept inline for type compatibility
// with the api.ts; new code should reach for the bridge's
// types instead — they have the same names but with snake-cased fields
// matching the Rust shape).

// Stream and Thread types moved to bindings — api.ts re-exports
// them directly from tauri-bridge/generated/bindings now. The
// legacy nested `panes` / `resume` sub-objects on Stream and the
// "active" | "queued" status restriction on Thread (which masked
// the bindings "closed" variant) are gone.

export interface ThreadState {
  selectedThreadId: string | null;
  activeThreadId: string | null;
  threads: import("./tauri-bridge/index.js").Thread[];
}

export interface TaskNote {
  id: string;
  task_id: string | null;
  thread_id: string | null;
  body: string;
  author: string;
  created_at: string;
}

// ---- Snapshots ----

// Snapshot interfaces (FileSnapshot, SnapshotSource, SnapshotEntry,
// SnapshotEntryState, SnapshotFileRow) live in api.ts now — the
// renderer-side aggregate surface is richer than the bindings
// shape (label, source enum, created_at) and is the version every
// consumer reads.

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

export interface GitWorktreeEntry {
  path: string;
  branch: string;
  head: string;
  isMain?: boolean;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}

export interface RemoteBranchEntry {
  name: string;
  ref: string;
  lastCommitAt: string | null;
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

// ---- Workspace ----

export interface WorkspaceFile {
  path: string;
  content: string;
}

export interface WorkspacePathChange {
  kind: "rename" | "delete" | "create" | "modify";
  path: string;
  toPath?: string;
}

export interface WorkspaceRenameResult {
  fromPath: string;
  toPath: string;
}

export interface WorkspaceWatchEvent {
  kind: "change" | "remove" | "create";
  path: string;
}

// ---- Agent statuses ----

export type AgentStatus = "running" | "idle" | "stopped" | "error";

// ---- MenuGroupSnapshot / CommandId placeholders ----

export type CommandId = string;
export interface MenuGroupSnapshot {
  id: string;
  label: string;
  items: { id: CommandId; label: string; disabled?: boolean }[];
}

// ---- Wiki notes ----

export interface WikiPageSummary {
  slug: string;
  title: string;
  excerpt: string;
  updated_at: string;
  /** Repo-relative file paths the page references (backend `file_refs`). */
  file_refs?: string[];
  dir_refs?: string[];
  // The Electron-era freshness/changed_refs/deleted_refs/total_refs/
  // referenced_files fields were removed — the Rust backend never sent
  // them, so readers silently rendered nothing. Per-page freshness
  // comes from `list_wiki_freshness` via `summarizeWikiFreshness`.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}

export interface WikiPageSearchHit {
  slug: string;
  title: string;
  snippet: string;
  updated_at: string;
}

// ---- Page visit / usage ----

export interface CountByDayRowApi {
  day: string;
  count: number;
}

export interface TopVisitedRowApi {
  pageKind: string;
  pageId: string;
  visitCount: number;
}

export interface OxplowConfig {
  agents: import("./tauri-bridge/generated/bindings.js").AgentKind[];
  projectName: string;
  agentPromptAppend: string;
  snapshotRetentionDays: number;
  snapshotMaxFileBytes: number;
  /** Extra ignore (`exclude`) / force-track (`include`) paths layered on
   *  top of `.gitignore`. `.git`/`.oxplow` and everything gitignored are
   *  ignored automatically. */
  generated: { exclude: string[]; include: string[] };
  injectSessionContext: boolean;
  /** The project's architectural zone table (`zones:` in
   *  .oxplow/project.yaml), in evaluation order — first match wins.
   *  Absent/empty means the project declared none, so every file
   *  classifies as `other` (tsk251). */
  zones?: import("./tauri-bridge/generated/bindings.js").ZoneRuleConfig[];
  /** Per-agent launch model overrides (`agentModels:` in .oxplow/project.yaml).
   *  Only opencode consumes its entry today. */
  agentModels?: Partial<Record<import("./tauri-bridge/generated/bindings.js").AgentKind, string>>;
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

export interface FinishedEntry {
  id: string;
  kind: string;
  finishedAt: string;
}

// ---- OxplowEvent (UI event-bus payloads) ----

// Permissive OxplowEvent shape — the original was a discriminated
// union; under Tauri we route events through the bridge with typed
// payloads, so this exists only for UI event-bus subscriber call
// sites. Each subscriber narrows on `type` and treats the rest of
// the fields as freeform; that compiles cleanly with this shape.
export interface OxplowEvent {
  type: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [key: string]: any;
}


// DesktopApi (the legacy permissive index-signature interface) was
// deleted: every renderer caller now reaches for either a typed
// top-level wrapper in api.ts or the small DesktopBridge facade
// returned by `desktopBridge()`. The window.oxplowApi global is
// long gone.
