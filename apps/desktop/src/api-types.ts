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

export type TaskKind = "epic" | "task" | "subtask" | "bug" | "note";
export type TaskStatus = "ready" | "in_progress" | "blocked" | "done" | "canceled" | "archived";
export type TaskPriority = "low" | "medium" | "high" | "urgent";

export interface Task {
  id: string;
  thread_id: string | null;
  parent_id: number | null;
  kind: TaskKind;
  title: string;
  description: string;
  status: TaskStatus;
  priority: TaskPriority;
  sort_index: number;
  created_by: "user" | "agent" | "system";
  created_at: string;
  updated_at: string;
  completed_at: string | null;
  note_count: number;
  author: "user" | "agent" | null;
}

export interface TaskNote {
  id: string;
  task_id: string | null;
  thread_id: string | null;
  body: string;
  author: string;
  created_at: string;
}

export interface TaskEvent {
  id: string;
  thread_id: string;
  item_id: string | null;
  event_type: string;
  actor_kind: "user" | "agent" | "system";
  actor_id: string;
  payload_json: string;
  created_at: string;
}

// ---- Snapshots ----

// Snapshot interfaces (FileSnapshot, SnapshotSource, SnapshotEntry,
// SnapshotEntryState, SnapshotFileRow) live in api.ts now — the
// renderer-side aggregate surface is richer than the bindings
// shape (label, source enum, created_at) and is the version every
// consumer reads.

export interface SnapshotSummary {
  files: import("./api.js").SnapshotFileRow[];
}

export type SnapshotDiffSide = "absent" | import("./api.js").SnapshotEntryState;

export interface SnapshotDiffResult {
  pathA: string;
  pathB: string;
  diff: string;
}

// ---- Backlog / efforts ----

export interface TaskEffort {
  id: string;
  tasksId: number;
  startedAt: string;
  endedAt: string | null;
}

export interface EffortDetail {
  effort: TaskEffort;
  files: { path: string; changeKind: string }[];
}

export interface ThreadFollowup {
  id: string;
  threadId: string;
  body: string;
  createdAt: string;
}

export interface ThreadWorkState {
  tasks: Task[];
  effortsInFlight: TaskEffort[];
  followups: ThreadFollowup[];
}

export interface BacklogState {
  items: Task[];
}

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

export interface BranchChanges {
  base?: string;
  ahead?: number;
  behind?: number;
  /// New shape (matches Rust `oxplow_git::BranchChanges`): the
  /// merge-base SHA between HEAD and the requested base ref. Either
  /// this or the legacy `base` is populated.
  base_ref?: string;
  merge_base?: string | null;
  files: BranchChangeEntry[];
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
export interface GitLogRef {
  ref: string;
  short: string;
  kind: "tag" | "branch" | "head";
}

export interface GitLogResult {
  commits: import("./tauri-bridge/index.js").GitLogCommit[];
  refs?: GitLogRef[];
  currentBranch?: string | null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}

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

export interface GitOpResult {
  ok: boolean;
  message?: string;
  stdout?: string;
  stderr?: string;
  exitCode?: number;
  signal?: string;
  sha?: string;
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

export interface WorkspacePathChange {
  kind: "rename" | "delete" | "create" | "modify";
  path: string;
  toPath?: string;
}

export interface WorkspaceRenameResult {
  fromPath: string;
  toPath: string;
}

export interface WorkspaceStatusSummary {
  modified: number;
  added: number;
  deleted: number;
  renamed: number;
  untracked: number;
  total: number;
}

export interface WorkspaceContext {
  rootPath: string;
  defaultBranch: string | null;
  isGitRepo: boolean;
}

export interface WorkspaceWatchEvent {
  kind: "change" | "remove" | "create";
  path: string;
}

// ---- Hook events / agent statuses ----

export type AgentStatus = "running" | "idle" | "stopped" | "error";

export interface StoredEvent {
  id: string;
  kind: string;
  streamId: string;
  threadId: string | null;
  payload: unknown;
  createdAt: string;
}

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

export type GitOperationKind = "merge" | "rebase" | "cherry-pick" | "revert";

export interface RepoConflictState {
  operation: GitOperationKind | null;
  conflictedCount: number;
}

export interface FinishedEntry {
  id: string;
  kind: string;
  finishedAt: string;
}

// ---- Code quality ----

export type CodeQualityTool = "metrics" | "duplication";
export type CodeQualityScope = "workspace" | "stream" | "codebase" | "diff";
export type CodeQualityScanStatus = "pending" | "running" | "done" | "failed";
export type CodeQualityFindingKind = "complexity" | "duplication" | "duplicate-block";

export interface CodeQualityScanRow {
  id: number;
  tool: CodeQualityTool;
  scope: CodeQualityScope;
  status: CodeQualityScanStatus;
  started_at?: string;
  error_message?: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [extra: string]: any;
}

export interface CodeQualityFindingRow {
  id: number;
  scanId: number;
  path: string;
  startLine: number;
  endLine: number;
  kind: CodeQualityFindingKind;
  metricValue: number;
  extra: Record<string, unknown> | null;
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
