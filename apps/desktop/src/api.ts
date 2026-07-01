import { commands } from "./tauri-bridge/generated/bindings.js";
import { listen, onRemoteReconnect, triggerRemoteResync } from "./tauri-bridge/transport.js";

export { onRemoteReconnect, triggerRemoteResync };
import { EVENT_CHANNELS } from "./tauri-bridge/channels.js";
import type { OxplowEvent } from "./api-types.js";
import { normalizeSnapshotId } from "./effort-snapshot.js";
import { ipcErrorMessage } from "./ipc-error.js";
import type {
  CommentIntent,
  CommentMessage,
  CommentStatus,
  CommentThread,
  DiffEndpoint,
  DiffEntry,
  LaunchInfo,
  RecentProjectView,
  SearchHit,
} from "./tauri-bridge/generated/bindings.js";

export type { SearchHit };

/// Convert the tauri-specta {status, data|error} envelope into a
/// plain promise return. Errors are usually IpcError objects with
/// message/code, but arg-deserialization failures and panics arrive as
/// a plain string — `ipcErrorMessage` surfaces the real reason verbatim
/// instead of collapsing to a generic "ipc error" (see ipc-error.ts).
function unwrap<T>(result: { status: "ok"; data: T } | { status: "error"; error: unknown }): T {
  if (result.status === "ok") return result.data;
  throw new Error(ipcErrorMessage(result.error));
}

/// Synthesize a success-shaped GitOpResult for void-returning Tauri
/// commands (gitAddPath / gitRestorePath / gitAppendToGitignore).
/// Renderer code expects a {success, stdout, stderr, status} shape
/// to decide whether to surface a toast. Since these commands either
/// succeed or throw, success here is unconditional.
function synthOk(): import("./tauri-bridge/index.js").GitOpResult {
  return { success: true, stdout: "", stderr: "", status: 0 };
}

/// Pure slug derivation: lowercase ASCII alphanumerics, runs of any
/// other character collapse to a single hyphen, leading/trailing
/// hyphens trimmed. Worktree slug is fixed at creation and never
/// changes, so the formatting needs to be conservative.
function slugifyTitle(title: string): string {
  const base = title
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return base.length > 0 ? base : `stream-${Date.now()}`;
}

/// Map the bindings BackgroundTask shape to the renderer's
/// flavor: dates as epoch-ms numbers (camelCase) and `result`
/// pre-parsed from the JSON-encoded `result_json`. Stays in
/// place because the renderer's task-list views still read
/// startedAt / endedAt / result directly.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function adaptBackgroundTask(t: any): any {
  if (!t) return t;
  let result: unknown = undefined;
  if (typeof t.result_json === "string" && t.result_json.length > 0) {
    try {
      result = JSON.parse(t.result_json);
    } catch {
      // ignore
    }
  }
  return {
    ...t,
    startedAt: typeof t.started_at === "number" ? t.started_at : Date.now(),
    endedAt: typeof t.ended_at === "number" ? t.ended_at : null,
    result,
  };
}

/// Desktop bridge facade: a small object that exposes the few
/// runtime IPC methods consumers reach for via
/// `desktopBridge().X(...)` (menu / lsp / terminal / external-url
/// / logUi / oxplow event subscription). The pre-migration
/// adapter exposed every Tauri command this way; today every
/// other call site is a top-level wrapper that hits the
/// `commands.X` surface directly, so this object is intentionally
/// narrow.
function buildBridge() {
  return {
    setNativeMenu: async (
      groups: import("./api-types.js").MenuGroupSnapshot[],
    ): Promise<void> => {
      try {
        unwrap(await commands.setNativeMenu(groups as never));
      } catch {
        // Don't break the UI if menu installation fails (e.g.
        // platform doesn't support a particular accelerator).
      }
    },
    onMenuCommand: (handler: (commandId: string) => void): (() => void) => {
      let stopped = false;
      const unlistenPromise = listen("menu:command", (e) => {
        if (stopped) return;
        const payload = e.payload as { id?: string } | null;
        if (payload?.id) handler(payload.id);
      });
      return () => {
        stopped = true;
        void unlistenPromise.then((u) => u());
      };
    },
    updateEditorFocus: async (_payload: unknown): Promise<void> => {
      // No-op: the daemon doesn't consume editor focus today.
    },
    logUi: async (entry: {
      clientId?: string;
      level: string;
      message: string;
      context?: unknown;
      timestamp?: string;
    }): Promise<void> => {
      try {
        unwrap(
          await commands.logUi({
            clientId: entry.clientId ?? null,
            level: entry.level,
            message: entry.message,
            context: entry.context !== undefined ? JSON.stringify(entry.context) : null,
            timestamp: entry.timestamp ?? null,
          }),
        );
      } catch {
        // Don't let a logging failure surface to callers.
      }
    },
    onLspEvent: (handler: (event: unknown) => void): (() => void) => {
      let stopped = false;
      // `listen` rejects when no transport is mounted (bun tests);
      // swallow so client construction never produces an unhandled
      // rejection.
      const unlistenPromise = listen(EVENT_CHANNELS.lsp, (e) => {
        if (stopped) return;
        handler(e.payload);
      }).catch(() => null);
      return () => {
        stopped = true;
        void unlistenPromise.then((u) => u?.());
      };
    },
    openTerminalSession: async (
      paneTarget: string,
      cols: number,
      rows: number,
      transportMode: string,
    ): Promise<{ sessionId: string; replayB64: string }> => {
      const result = unwrap(
        await commands.openTerminalSession(paneTarget, cols, rows, transportMode),
      );
      return { sessionId: result.sessionId, replayB64: result.replayB64 };
    },
    closeTerminalSession: async (sessionId: string): Promise<void> => {
      try {
        unwrap(await commands.closeTerminalSession(sessionId));
      } catch {
        // Idempotent close.
      }
    },
    /// Permanently kill the PTY behind `sessionId` (vs `closeTerminalSession`,
    /// which only detaches and leaves the shell running). Used when a
    /// terminal tab is explicitly closed.
    terminateTerminalSession: async (sessionId: string): Promise<void> => {
      try {
        unwrap(await commands.terminateTerminalSession(sessionId));
      } catch {
        // Idempotent terminate.
      }
    },
    // Plumbing for HUMAN terminal input only — the xterm in TerminalPane
    // pipes the user's keystrokes / paste / scroll / resize through here.
    // NOT an agent-messaging or automation API; never synthesize
    // `{type:"input"}` from non-UI code (see .context/agent-model.md).
    forwardTerminalInput: async (sessionId: string, message: string): Promise<void> => {
      unwrap(await commands.forwardTerminalInput(sessionId, message));
    },
    /// Best-effort live cwd of the session's child (the shell, for the Terminal
    /// page). null when undeterminable; callers fall back to the worktree root.
    terminalSessionCwd: async (sessionId: string): Promise<string | null> => {
      try {
        return unwrap(await commands.terminalSessionCwd(sessionId));
      } catch {
        return null;
      }
    },
    onTerminalEvent: (
      handler: (event: { sessionId: string; message: string }) => void,
    ): (() => void) => {
      let stopped = false;
      const unlistenPromise = listen(EVENT_CHANNELS.terminal, (e) => {
        if (stopped) return;
        handler(e.payload as { sessionId: string; message: string });
      });
      return () => {
        stopped = true;
        void unlistenPromise.then((u) => u());
      };
    },
    openExternalUrl: async (
      url: string,
    ): Promise<{ ok: boolean; reason?: string }> => {
      try {
        unwrap(await commands.openExternalUrl(url));
        return { ok: true };
      } catch (e) {
        return { ok: false, reason: e instanceof Error ? e.message : String(e) };
      }
    },
    /// `clipboardReadText` is read by `TerminalPane`'s legacy
    /// Electron-paste path; on Tauri the native clipboard shim is
    /// preferred so this can return null and the caller falls back.
    clipboardReadText: async (): Promise<string> =>
      unwrap(await commands.clipboardReadText()),
  };
}

export type DesktopBridge = ReturnType<typeof buildBridge>;
let cachedBridge: DesktopBridge | null = null;

export type { OxplowEvent } from "./api-types.js";
// Use the tauri-specta-generated shapes directly for the
// snake_case-native bindings (CommitDetail, GitLogCommit,
// RemoteBranchEntry, GitOpResult, BlameLine, …). The api-types
// camelCase legacy definitions were drifting from runtime shape
// and only existed because the original Electron build wrapped
// them in adapters; nothing converts shape today.
// Bindings shapes for the types whose call sites have been
// migrated. Adding more is a per-call-site refactor: each consumer
// has to be updated to the new field names. Types not on this list
// stay on the api-types camelCase legacy shape until their consumers
// are migrated.
export type {
  GitOpResult,
  GitWorktreeEntry,
  RemoteBranchEntry,
  GitLogCommit,
  CommitDetail,
  BlameLine,
  StreamDivergenceReport,
  StreamDivergenceRow,
  MergeReadiness,
} from "./tauri-bridge/index.js";
// The remaining legacy types still come from api-types because
// their consumers read fields that don't exist on the bindings
// shape yet (e.g. GitLogResult.currentBranch / branchHeads / tags,
// RemoteBranchEntry.remote / branch / lastCommitDate, GitWorktreeEntry
// camelCase aliases, BranchChangeEntry.status / additions / deletions
// — bindings expose .change and don't surface line counts here yet).
// Migrating each one is per-call-site work; until then the shape
// the runtime hands the renderer is the bindings shape but the
// renderer's TypeScript believes it's the legacy shape.
export type {
  GitLogRef,
  GitLogResult,
  ChangeScopes,
  TextSearchHit,
  RefOption,
  GroupedGitRefs,
  BranchChangeEntry,
  BranchChanges,
} from "./api-types.js";

// Stream / Thread come straight from the Tauri bindings — the
// renderer reads the flat shape (working_pane / talking_pane /
// custom_prompt) directly; no synthesis happens at the boundary.
import type { AgentKind, Stream, Thread } from "./tauri-bridge/index.js";
export type { AgentKind, Stream, Thread };

export interface ThreadState {
  selectedThreadId: string | null;
  activeThreadId: string | null;
  threads: Thread[];
}

// tasks types now come from the Tauri bindings. The bindings
// emit a `deleted_at` field that the earlier UI interface didn't model;
// readers either ignore it or filter on it (earlier stores already
// excluded soft-deleted rows in their list queries). New code can
// read `deleted_at` directly when needed.
import type {
  Task,
  TaskStatus,
  TaskPriority,
} from "./tauri-bridge/index.js";
export type { Task, TaskStatus, TaskPriority };

export interface TaskNote {
  id: string;
  task_id: string;
  body: string;
  author: string;
  created_at: string;
}

import type { TaskEvent } from "./tauri-bridge/index.js";
export type { TaskEvent };

export type SnapshotSource =
  | "effort-start"
  | "effort-end"
  | "effort-event"
  | "startup"
  | "manual"
  | "git-refs";

export interface FileSnapshot {
  id: string;
  stream_id: string;
  worktree_path: string;
  version_hash: string;
  source: SnapshotSource;
  created_at: string;
  label?: string | null;
  label_kind?: "task" | "turn" | "system" | null;
}

export type SnapshotEntryState = "present" | "oversize";

export interface SnapshotEntry {
  hash: string;
  mtime_ms: number;
  size: number;
  state: SnapshotEntryState;
}

export interface SnapshotFileRow {
  entry: SnapshotEntry;
  kind: "created" | "updated" | "deleted";
}

export interface SnapshotSummary {
  snapshot: FileSnapshot;
  previousSnapshotId: string | null;
  files: Record<string, SnapshotFileRow>;
  counts: { created: number; updated: number; deleted: number };
}

export type SnapshotDiffSide = "absent" | SnapshotEntryState;

export interface SnapshotDiffResult {
  before: string | null;
  after: string | null;
  beforeState: SnapshotDiffSide;
  afterState: SnapshotDiffSide;
}

export interface TaskEffort {
  id: string;
  task_id: string;
  started_at: string;
  ended_at: string | null;
  start_snapshot_id: string | null;
  end_snapshot_id: string | null;
  /** The effort's summary prose (canonical text). */
  summary: string | null;
}

export interface EffortDetail {
  effort: TaskEffort;
  start_snapshot: FileSnapshot | null;
  end_snapshot: FileSnapshot | null;
  changed_paths: string[];
  counts: { created: number; updated: number; deleted: number };
}

// Followup is bindings.Followup; ThreadWorkState is the bundle the
// Work panel renders. Both are emitted by tauri-specta now.
import type { Followup as ThreadFollowup, ThreadWorkState as TauriThreadWorkState, BacklogState as TauriBacklogState } from "./tauri-bridge/index.js";
export type { ThreadFollowup };
export type ThreadWorkState = TauriThreadWorkState;
export type BacklogState = TauriBacklogState;

export const BACKLOG_SCOPE = "__backlog__";

export interface BranchRef {
  kind: "local" | "remote";
  name: string;
  ref: string;
  remote?: string;
}

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

export interface WorkspacePathChange {
  path: string;
}

export interface WorkspaceRenameResult {
  fromPath: string;
  toPath: string;
}

export interface WorkspaceIndexedFile {
  path: string;
  gitStatus: GitFileStatus | null;
}

import type { WorkspaceStatusSummary } from "./tauri-bridge/index.js";
import type { InstalledLspPackage, LspServerListing } from "./tauri-bridge/generated/bindings.js";
export type { InstalledLspPackage, LspServerListing };
export type { WorkspaceStatusSummary };

export interface WorkspaceContext {
  gitEnabled: boolean;
}

export interface WorkspaceWatchEvent {
  id: number;
  streamId: string;
  path: string;
  kind: "created" | "updated" | "deleted";
  t: number;
}

// Stream + config wrappers. Each call goes straight to the
// tauri-specta `commands` surface — no buildDesktopAdapter
// detour. The unwrap() helper at the top of this file converts
// the {status, data|error} envelope into a plain promise.

export async function listStreams(): Promise<Stream[]> {
  return unwrap(await commands.listStreams());
}

/// Site-wide BM25 search. `streamId` scopes file/stream-bound hits to one
/// worktree (project-global hits like wiki always included); `null` searches
/// everything. `kinds` optionally restricts to task|comment|note|wiki|file.
export async function searchSite(
  query: string,
  streamId: string | null,
  kinds: string[] | null = null,
  limit = 50,
): Promise<SearchHit[]> {
  return unwrap(await commands.search(query, streamId, kinds, limit));
}

export async function listThreads(streamId: string): Promise<Thread[]> {
  return unwrap(await commands.listThreads(streamId)) as unknown as Thread[];
}

export async function getCurrentStream(): Promise<Stream> {
  const cur = unwrap(await commands.getCurrentStream());
  if (cur) return cur;
  const primary = unwrap(await commands.getPrimaryStream());
  if (!primary) throw new Error("no primary stream available");
  return primary;
}

export async function switchStream(id: string): Promise<Stream> {
  unwrap(await commands.switchStream(id));
  return getCurrentStream();
}

export async function renameStream(streamId: string, title: string): Promise<Stream> {
  return unwrap(await commands.renameStream({ id: streamId, title }));
}

export async function archiveStream(streamId: string, deleteWorktree: boolean): Promise<void> {
  unwrap(await commands.archiveStream(streamId, deleteWorktree));
}

export async function renameCurrentStream(title: string): Promise<Stream> {
  const cur = unwrap(await commands.getCurrentStream());
  if (!cur) throw new Error("no current stream to rename");
  return renameStream(cur.id, title);
}

export async function getConfig(): Promise<import("./api-types.js").OxplowConfig> {
  return unwrap(await commands.getConfig()) as unknown as import("./api-types.js").OxplowConfig;
}

export async function setAgents(agents: AgentKind[]): Promise<import("./api-types.js").OxplowConfig> {
  return unwrap(await commands.setAgents(agents)) as unknown as import("./api-types.js").OxplowConfig;
}

export async function setAgentPromptAppend(text: string): Promise<import("./api-types.js").OxplowConfig> {
  return unwrap(await commands.setAgentPromptAppend(text)) as unknown as import("./api-types.js").OxplowConfig;
}

export async function setGenerated(
  generated: { exclude: string[]; include: string[] },
): Promise<import("./api-types.js").OxplowConfig> {
  return unwrap(await commands.setGenerated(generated)) as unknown as import("./api-types.js").OxplowConfig;
}

/// Set (or clear, with null/blank) the launch-model override for one
/// agent — `agentModels.<agent>` in .oxplow/project.yaml. Only opencode consumes
/// the override today (`opencode -m provider/model`).
export async function setAgentModel(
  agent: AgentKind,
  model: string | null,
): Promise<import("./api-types.js").OxplowConfig> {
  return unwrap(await commands.setAgentModel(agent, model)) as unknown as import("./api-types.js").OxplowConfig;
}

export async function setSnapshotRetentionDays(days: number): Promise<import("./api-types.js").OxplowConfig> {
  return unwrap(await commands.setSnapshotRetentionDays(days)) as unknown as import("./api-types.js").OxplowConfig;
}

export async function setSnapshotMaxFileBytes(bytes: number): Promise<import("./api-types.js").OxplowConfig> {
  return unwrap(await commands.setSnapshotMaxFileBytes(bytes)) as unknown as import("./api-types.js").OxplowConfig;
}

export async function listBranches(): Promise<BranchRef[]> {
  return unwrap(await commands.listLocalBranches()) as unknown as BranchRef[];
}

export async function getDefaultBranch(): Promise<string | null> {
  return unwrap(await commands.getDefaultBranch());
}

export type CommitRefLabel = import("./tauri-bridge/generated/bindings.js").CommitRefLabel;

export async function resolveCommitRefLabels(
  shas: string[],
): Promise<Record<string, CommitRefLabel[]>> {
  if (shas.length === 0) return {};
  return unwrap(await commands.resolveCommitRefLabels(shas));
}

export async function listGitRefs(): Promise<import("./api-types.js").GroupedGitRefs> {
  const raw = unwrap(await commands.listAllRefs());
  const localBranches = raw.locals.map((r) => ({
    kind: "local" as const,
    name: r.label,
    ref: r.ref,
  }));
  const byRemote = new Map<
    string,
    Array<{ kind: "remote"; name: string; ref: string; remote: string }>
  >();
  for (const r of raw.remotes) {
    const slash = r.label.indexOf("/");
    const remote = slash >= 0 ? r.label.slice(0, slash) : "origin";
    const name = slash >= 0 ? r.label.slice(slash + 1) : r.label;
    if (!byRemote.has(remote)) byRemote.set(remote, []);
    byRemote.get(remote)!.push({ kind: "remote", name, ref: r.ref, remote });
  }
  return {
    local: localBranches,
    remote: Array.from(byRemote.values()).flat(),
    remotes: Array.from(byRemote.entries()).map(([remote, branches]) => ({
      remote,
      branches,
    })),
    tags: raw.tags.map((t) => ({ name: t.label, ref: t.ref })),
    recent: localBranches.slice(0, 5).map((b) => b.name),
  } as unknown as import("./api-types.js").GroupedGitRefs;
}

export async function renameGitBranch(
  from: string,
  to: string,
): Promise<import("./tauri-bridge/index.js").GitOpResult> {
  unwrap(await commands.renameBranch(from, to));
  return synthOk();
}

export async function deleteGitBranch(
  branch: string,
  options?: { force?: boolean },
): Promise<import("./tauri-bridge/index.js").GitOpResult> {
  unwrap(await commands.deleteBranch(branch, options?.force ?? false));
  return synthOk();
}

/**
 * Long-running git ops are kickoff-style — the IPC promise resolves
 * immediately with a `taskId` once the BackgroundTaskStore row is
 * registered, and the actual work runs in the background. Each
 * renderer-side wrapper also exposes an `awaitDone` promise that
 * resolves with the final `BackgroundTask` (status, error, and
 * `result` payload — typically a `GitOpResult`). Pattern:
 *
 *     const { taskId, awaitDone } = await gitRebaseOnto(...);
 *     // mark UI pending using taskId / a label
 *     const task = await awaitDone;
 *     // task.result is the GitOpResult
 *
 * Callers that don't need the final result can ignore `awaitDone`;
 * any other surface watching `subscribeBackgroundTaskEvents` still
 * sees the same in-flight state.
 */
export interface GitOpKickoff {
  taskId: string;
  awaitDone: Promise<BackgroundTask | null>;
}

function attachAwait(taskId: string): GitOpKickoff {
  return { taskId, awaitDone: awaitBackgroundTask(taskId) };
}

/// Wrap a synchronous Tauri git op inside a real BackgroundTask
/// row so `awaitDone` resolves with the actual GitOpResult and the
/// shared "in-flight task" subscribers stay accurate. Without
/// this, the renderer's kickoff pattern (gitPush / gitPull etc.)
/// would race a never-completing fake task and the result would
/// land in the void.
async function runAsBackgroundTask(
  label: string,
  kind: import("./tauri-bridge/index.js").BackgroundTaskKind,
  detail: string | null,
  op: () => Promise<import("./tauri-bridge/index.js").GitOpResult>,
): Promise<GitOpKickoff> {
  const task = unwrap(await commands.startBackgroundTask(kind, label, detail));
  const taskId = task.id;
  void (async () => {
    try {
      const result = await op();
      unwrap(
        await commands.completeBackgroundTask(taskId, JSON.stringify(result)),
      );
    } catch (err) {
      unwrap(
        await commands.failBackgroundTask(
          taskId,
          err instanceof Error ? err.message : String(err),
        ),
      );
    }
  })();
  return attachAwait(taskId);
}

export async function gitMergeInto(streamId: string, other: string): Promise<GitOpKickoff> {
  return runAsBackgroundTask(`Merge ${other}`, "git", `merge ${other}`, async () =>
    unwrap(await commands.gitMergeInto(streamId, other)),
  );
}

export async function gitRebaseOnto(streamId: string, onto: string): Promise<GitOpKickoff> {
  return runAsBackgroundTask(`Rebase onto ${onto}`, "git", `rebase ${onto}`, async () =>
    unwrap(await commands.gitRebaseOnto(streamId, onto)),
  );
}

export async function gitCherryPick(streamId: string, commit: string): Promise<GitOpKickoff> {
  const short = commit.slice(0, 7);
  return runAsBackgroundTask(`Cherry-pick ${short}`, "git", `cherry-pick ${short}`, async () =>
    unwrap(await commands.gitCherryPick(streamId, commit)),
  );
}

export async function gitRevert(streamId: string, commit: string): Promise<GitOpKickoff> {
  const short = commit.slice(0, 7);
  return runAsBackgroundTask(`Revert ${short}`, "git", `revert ${short}`, async () =>
    unwrap(await commands.gitRevert(streamId, commit)),
  );
}

export async function getWorkspaceContext(): Promise<WorkspaceContext> {
  const ctx = unwrap(await commands.getWorkspaceContext());
  return { gitEnabled: ctx.is_git_repo };
}

// ---- Launcher / multi-window (process-per-window) ----

/// Whether this process booted into the launcher or a project. The
/// `<Root>` gate calls this first to pick the top-level screen.
export async function getLaunchMode(): Promise<LaunchInfo> {
  return unwrap(await commands.getLaunchMode());
}

/// Recent projects for the launcher, most-recent first, each tagged
/// with whether its directory still exists on disk.
export async function listRecentProjects(): Promise<RecentProjectView[]> {
  return unwrap(await commands.listRecentProjects());
}

/// Forget a project from the recent list.
export async function removeRecentProject(path: string): Promise<void> {
  unwrap(await commands.removeRecentProject(path));
}

/// Open `path` as a project. `newWindow=false` replaces the current
/// window (this process exits once the new one is spawned);
/// `newWindow=true` opens an additional independent window.
export async function openProject(path: string, newWindow: boolean): Promise<void> {
  unwrap(await commands.openProject(path, newWindow));
}

/// Whether `path` still needs first-run setup (has no `.oxplow/` yet).
export async function projectNeedsSetup(path: string): Promise<boolean> {
  return unwrap(await commands.projectNeedsSetup(path));
}

/// Create the `.oxplow/` project structure in `path` and relaunch into
/// it. Called from the setup-confirmation screen.
export async function setupProject(path: string): Promise<void> {
  unwrap(await commands.setupProject(path));
}

/// Decline first-run setup — closes the setup window (exits the process).
export async function abortSetup(): Promise<void> {
  unwrap(await commands.abortSetup());
}

/// Open `path`, but route uninitialized dirs (no `.oxplow/`) to a NEW
/// window regardless of `newWindow` — that window shows the setup
/// confirmation, so declining it can never destroy the launcher or the
/// caller's current project window.
export async function openProjectGuarded(path: string, newWindow: boolean): Promise<void> {
  const needsSetup = await projectNeedsSetup(path);
  await openProject(path, needsSetup ? true : newWindow);
}

export async function createStream(input:
  | { title: string; source: "existing"; ref: string }
  | { title: string; source: "new"; branch: string; startPointRef: string }
  | { title: string; source: "worktree"; worktreePath: string },
): Promise<Stream> {
  const slug = slugifyTitle(input.title);
  switch (input.source) {
    case "existing":
      return unwrap(
        await commands.createWorktree({
          slug,
          title: input.title,
          branch: input.ref,
          branchSource: input.ref,
        }),
      );
    case "new":
      return unwrap(
        await commands.createWorktree({
          slug,
          title: input.title,
          branch: input.branch,
          branchSource: input.startPointRef ?? input.branch,
        }),
      );
    case "worktree":
      return unwrap(
        await commands.adoptWorktree({
          path: input.worktreePath,
          title: input.title,
        }),
      );
  }
}

export async function listAdoptableWorktrees(): Promise<
  import("./tauri-bridge/index.js").GitWorktreeEntry[]
> {
  return unwrap(await commands.listAdoptableWorktrees());
}

export async function checkoutStreamBranch(streamId: string, branch: string): Promise<Stream> {
  return unwrap(await commands.checkoutStreamBranch(streamId, branch));
}

export async function getThreadState(streamId: string): Promise<ThreadState> {
  return unwrap(await commands.getThreadState(streamId)) as unknown as ThreadState;
}

export async function createThread(streamId: string, title: string, agent?: AgentKind): Promise<ThreadState> {
  unwrap(
    await commands.createThread({ streamId, title, paneTarget: null, agent: agent ?? null }),
  );
  return getThreadState(streamId);
}

export async function reorderThreads(streamId: string, orderedThreadIds: string[]): Promise<void> {
  unwrap(
    await commands.reorderThreadQueue({ streamId, order: orderedThreadIds }),
  );
}

export async function reorderStreams(orderedStreamIds: string[]): Promise<void> {
  unwrap(await commands.reorderStreams(orderedStreamIds));
}

export async function selectThread(streamId: string, threadId: string): Promise<ThreadState> {
  unwrap(await commands.selectThread({ streamId, threadId }));
  return getThreadState(streamId);
}

export async function promoteThread(streamId: string, threadId: string): Promise<ThreadState> {
  unwrap(await commands.promoteThread(threadId));
  return getThreadState(streamId);
}

export async function closeThread(streamId: string, threadId: string): Promise<ThreadState> {
  unwrap(await commands.closeThread(threadId));
  return getThreadState(streamId);
}

export async function reopenThread(streamId: string, threadId: string): Promise<ThreadState> {
  unwrap(await commands.reopenThread(threadId));
  return getThreadState(streamId);
}

export async function listClosedThreads(streamId: string): Promise<Thread[]> {
  return unwrap(await commands.listClosedThreads(streamId));
}

export async function renameThread(_streamId: string, threadId: string, title: string): Promise<Thread> {
  return unwrap(await commands.renameThread({ id: threadId, title }));
}

export async function setStreamPrompt(streamId: string, prompt: string | null): Promise<Stream[]> {
  unwrap(await commands.setStreamPrompt({ id: streamId, prompt }));
  return listStreams();
}

export async function setThreadPrompt(
  _streamId: string,
  threadId: string,
  prompt: string | null,
): Promise<Thread[]> {
  unwrap(await commands.setThreadPrompt({ id: threadId, prompt }));
  return [];
}

export async function getThreadWorkState(_streamId: string, threadId: string): Promise<ThreadWorkState> {
  return unwrap(await commands.getThreadWorkState(threadId)) as unknown as ThreadWorkState;
}

export async function createTask(
  streamId: string,
  threadId: string,
  input: {
    title: string;
    description?: string;
    parentId?: number | null;
    status?: TaskStatus;
    priority?: TaskPriority;
  },
): Promise<ThreadWorkState> {
  unwrap(await commands.createTask({ threadId, input: input as never }));
  return getThreadWorkState(streamId, threadId);
}

export async function updateTask(
  streamId: string,
  threadId: string,
  itemId: string,
  changes: {
    title?: string;
    description?: string;
    parentId?: string | null;
    status?: TaskStatus;
    priority?: TaskPriority;
  },
): Promise<ThreadWorkState> {
  unwrap(await commands.updateTask({ id: itemId, changes: changes as never }));
  return getThreadWorkState(streamId, threadId);
}

export async function deleteTask(
  streamId: string,
  threadId: string,
  itemId: string,
): Promise<ThreadWorkState> {
  unwrap(await commands.deleteTask(itemId));
  return getThreadWorkState(streamId, threadId);
}

export async function reorderTasks(
  streamId: string,
  threadId: string,
  orderedItemIds: string[],
): Promise<ThreadWorkState> {
  unwrap(await commands.reorderTasks({ threadId, order: orderedItemIds }));
  return getThreadWorkState(streamId, threadId);
}

export async function moveTaskToThread(
  streamId: string,
  fromThreadId: string,
  itemId: string,
  toThreadId: string,
  _toStreamId?: string,
): Promise<{ from: ThreadWorkState; to: ThreadWorkState }> {
  unwrap(await commands.moveTask({ id: itemId, threadId: toThreadId }));
  const [from, to] = await Promise.all([
    getThreadWorkState(streamId, fromThreadId),
    getThreadWorkState(streamId, toThreadId),
  ]);
  return { from, to };
}

export async function getBacklogState(): Promise<BacklogState> {
  return unwrap(await commands.getBacklogState()) as unknown as BacklogState;
}

export async function createBacklogItem(input: {
  title: string;
  description?: string;
  status?: TaskStatus;
  priority?: TaskPriority;
}): Promise<BacklogState> {
  unwrap(await commands.createTask({ threadId: null, input: input as never }));
  return getBacklogState();
}

export async function updateBacklogItem(
  itemId: string,
  changes: {
    title?: string;
    description?: string;
    status?: TaskStatus;
    priority?: TaskPriority;
  },
): Promise<BacklogState> {
  unwrap(await commands.updateTask({ id: itemId, changes: changes as never }));
  return getBacklogState();
}

export async function deleteBacklogItem(itemId: string): Promise<BacklogState> {
  unwrap(await commands.deleteTask(itemId));
  return getBacklogState();
}

export async function reorderBacklog(orderedItemIds: string[]): Promise<BacklogState> {
  unwrap(await commands.reorderTasks({ threadId: null, order: orderedItemIds }));
  return getBacklogState();
}

export async function moveTaskToBacklog(
  streamId: string,
  fromThreadId: string,
  itemId: string,
): Promise<{ from: ThreadWorkState; backlog: BacklogState }> {
  unwrap(await commands.moveTask({ id: itemId, threadId: null }));
  const [from, backlog] = await Promise.all([
    getThreadWorkState(streamId, fromThreadId),
    getBacklogState(),
  ]);
  return { from, backlog };
}

export async function moveBacklogItemToThread(
  streamId: string,
  itemId: string,
  toThreadId: string,
): Promise<{ backlog: BacklogState; to: ThreadWorkState }> {
  unwrap(await commands.moveTask({ id: itemId, threadId: toThreadId }));
  const [backlog, to] = await Promise.all([
    getBacklogState(),
    getThreadWorkState(streamId, toThreadId),
  ]);
  return { backlog, to };
}

export async function getGitLog(
  streamId: string,
  options?: { limit?: number; all?: boolean },
): Promise<import("./api-types.js").GitLogResult> {
  const raw = unwrap(
    await commands.getGitLog(streamId, options?.limit ?? null, options?.all ?? false),
  );
  return raw as unknown as import("./api-types.js").GitLogResult;
}

export async function getCommitDetail(
  streamId: string,
  sha: string,
): Promise<import("./tauri-bridge/index.js").CommitDetail | null> {
  return unwrap(await commands.getCommitDetail(streamId, sha));
}

export async function getChangeScopes(
  streamId: string,
): Promise<import("./api-types.js").ChangeScopes> {
  const raw = unwrap(await commands.getChangeScopes(streamId));
  return {
    staged: raw.staged as unknown as import("./api-types.js").BranchChangeEntry[],
    unstaged: raw.unstaged as unknown as import("./api-types.js").BranchChangeEntry[],
    currentBranch: raw.current_branch ?? undefined,
    branchBase: raw.branch_base ?? undefined,
    upstream: raw.upstream ?? undefined,
    onDefaultBranch: raw.on_default_branch,
  };
}

export async function searchWorkspaceText(
  streamId: string,
  query: string,
  options?: { limit?: number },
): Promise<import("./api-types.js").TextSearchHit[]> {
  return unwrap(
    await commands.searchWorkspaceText(streamId, query, options?.limit ?? null),
  ) as unknown as import("./api-types.js").TextSearchHit[];
}

export async function gitRestorePath(
  streamId: string,
  path: string,
): Promise<import("./tauri-bridge/index.js").GitOpResult> {
  unwrap(await commands.restorePath(streamId, path));
  return synthOk();
}

export async function gitAddPath(
  streamId: string,
  path: string,
): Promise<import("./tauri-bridge/index.js").GitOpResult> {
  unwrap(await commands.gitAddPath(streamId, path));
  return synthOk();
}

export async function gitAppendToGitignore(
  streamId: string,
  path: string,
): Promise<import("./tauri-bridge/index.js").GitOpResult> {
  unwrap(await commands.appendToGitignore(streamId, path));
  return synthOk();
}

export async function gitPush(
  streamId: string,
  _options?: { force?: boolean; setUpstream?: boolean; remote?: string; branch?: string },
): Promise<GitOpKickoff> {
  return runAsBackgroundTask("Push", "git", "git push", async () =>
    unwrap(await commands.gitPush(streamId)),
  );
}

export async function gitPull(
  streamId: string,
  _options?: { rebase?: boolean; remote?: string; branch?: string },
): Promise<GitOpKickoff> {
  return runAsBackgroundTask("Pull", "git", "git pull", async () =>
    unwrap(await commands.gitPull(streamId)),
  );
}

export async function gitFetch(
  streamId: string,
  options?: { remote?: string; prune?: boolean; all?: boolean },
): Promise<GitOpKickoff> {
  const remote = options?.remote ?? null;
  return runAsBackgroundTask("Fetch", "git", `git fetch${remote ? ` ${remote}` : ""}`, async () =>
    unwrap(await commands.gitFetch(streamId, remote)),
  );
}

export async function gitCommitAll(
  streamId: string,
  message: string,
  _options?: { includeUntracked?: boolean; paths?: string[] },
): Promise<import("./tauri-bridge/index.js").GitOpResult & { sha?: string }> {
  return unwrap(await commands.gitCommitAll(streamId, message));
}

export async function getAheadBehind(
  streamId: string,
  base: string,
  head?: string,
): Promise<{ ahead: number; behind: number }> {
  const ab = unwrap(await commands.getAheadBehind(streamId, base, head ?? "HEAD"));
  return { ahead: ab.ahead, behind: ab.behind };
}

export async function listStreamDivergences(
  base?: string,
): Promise<import("./tauri-bridge/index.js").StreamDivergenceReport> {
  return unwrap(await commands.listStreamDivergences(base ?? null));
}

export async function getCommitsAheadOf(
  streamId: string,
  base: string,
  head: string,
  limit?: number,
): Promise<import("./tauri-bridge/index.js").GitLogCommit[]> {
  return unwrap(
    await commands.getCommitsAheadOf(streamId, base, head, limit ?? 200),
  );
}

export async function listRecentRemoteBranches(
  _streamId: string,
  limit?: number,
): Promise<import("./tauri-bridge/index.js").RemoteBranchEntry[]> {
  return unwrap(await commands.listRecentRemoteBranches(limit ?? null));
}

export async function gitPushCurrentTo(
  streamId: string,
  remote: string,
  branch: string,
): Promise<GitOpKickoff> {
  return runAsBackgroundTask(
    `Push to ${remote}/${branch}`,
    "git",
    `git push ${remote} ${branch}`,
    async () => unwrap(await commands.gitPushCurrentTo(streamId, remote, branch)),
  );
}

export async function gitPullRemoteIntoCurrent(
  streamId: string,
  remote: string,
  branch: string,
): Promise<GitOpKickoff> {
  return runAsBackgroundTask(
    `Pull ${remote}/${branch} into current`,
    "git",
    `git pull ${remote} ${branch}`,
    async () => unwrap(await commands.gitPullRemoteIntoCurrent(streamId, remote, branch)),
  );
}

export async function listFileCommits(
  streamId: string,
  path: string,
  limit?: number,
): Promise<import("./tauri-bridge/index.js").GitLogCommit[]> {
  return unwrap(await commands.listFileCommits(streamId, path, limit ?? null));
}

export async function gitBlame(
  streamId: string,
  path: string,
): Promise<import("./tauri-bridge/index.js").BlameLine[]> {
  return unwrap(await commands.gitBlame(streamId, path));
}

/// Renderer-side LocalBlameEntry: the bindings shape plus an
/// optional `tasks` overlay the editor's blame margin paints
/// when a snapshot/tasks attribution exists. The runtime
/// today only populates {line, source, git}; `tasks` arrives
/// once the snapshot blob store grows attribution lookup. Until
/// then the editor's local-blame branch is dormant but typesafe.
export interface LocalBlameEntry {
  line: number;
  source: string;
  git: import("./tauri-bridge/index.js").BlameLine | null;
  tasks?: {
    id: string;
    title: string;
    endedAt: string;
  };
}

export async function localBlame(
  streamId: string,
  path: string,
): Promise<LocalBlameEntry[]> {
  return unwrap(
    await commands.localBlame(streamId, path, ""),
  ) as unknown as LocalBlameEntry[];
}

export type WikiPageSummary = import("./api-types.js").WikiPageSummary;
export type WikiPageSearchHit = import("./api-types.js").WikiPageSearchHit;
export type UsageRollup = import("./tauri-bridge/generated/bindings.js").UsageRollup;

export async function listWikiPages(_streamId: string): Promise<WikiPageSummary[]> {
  return unwrap(await commands.listWikiPages()) as unknown as WikiPageSummary[];
}

export async function readWikiPageBody(_streamId: string, slug: string): Promise<string> {
  return unwrap(await commands.readWikiPageBody(slug));
}

export async function writeWikiPageBody(_streamId: string, slug: string, body: string): Promise<void> {
  unwrap(await commands.writeWikiPageBody(slug, body));
}

export async function deleteWikiPage(_streamId: string, slug: string): Promise<void> {
  unwrap(await commands.deleteWikiPage(slug));
}

export function subscribeWikiPageEvents(onEvent: (slug: string) => void): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind === "wikiPagesChanged") {
      const slug = typeof event.slug === "string" ? event.slug : "";
      onEvent(slug);
    }
  });
}

// ---- Comments ----

export async function createComment(input: {
  streamId: string;
  threadId: string | null;
  targetKind: string;
  targetId: string;
  quote: string;
  /** W3C selectors array, serialized. */
  selectorsJson: string;
  /** Ancestor regions (innermost→outermost, excluding the target). */
  contextChain?: { kind: string; id: string }[];
  /** Canonical refs found inside the selection. */
  referencedRefs?: { kind: string; id: string }[];
  intent: CommentIntent;
  author: string;
  body: string;
}): Promise<CommentThread> {
  return unwrap(
    await commands.createComment({
      streamId: input.streamId,
      threadId: input.threadId,
      targetKind: input.targetKind,
      targetId: input.targetId,
      quote: input.quote,
      selectorsJson: input.selectorsJson,
      contextChain: input.contextChain ?? [],
      referencedRefs: input.referencedRefs ?? [],
      intent: input.intent,
      author: input.author,
      body: input.body,
    }),
  );
}

export async function addCommentMessage(
  commentId: string,
  author: string,
  body: string,
): Promise<CommentMessage> {
  return unwrap(await commands.addCommentMessage(commentId, author, body));
}

export async function listCommentsForTarget(
  targetKind: string,
  targetId: string,
): Promise<CommentThread[]> {
  return unwrap(await commands.listCommentsForTarget(targetKind, targetId));
}

export async function listCommentsForStream(streamId: string): Promise<CommentThread[]> {
  return unwrap(await commands.listCommentsForStream(streamId));
}

export async function setCommentIntent(commentId: string, intent: CommentIntent): Promise<void> {
  unwrap(await commands.setCommentIntent(commentId, intent));
}

export async function setCommentStatus(commentId: string, status: CommentStatus): Promise<void> {
  unwrap(await commands.setCommentStatus(commentId, status));
}

export async function setCommentAnchor(
  commentId: string,
  selectorsJson: string,
  orphaned: boolean,
): Promise<void> {
  unwrap(await commands.setCommentAnchor(commentId, selectorsJson, orphaned));
}

/// Re-attach an orphaned comment to a freshly-selected span: rewrites
/// both quote + anchor and clears the orphan flag.
export async function relinkComment(
  commentId: string,
  quote: string,
  selectorsJson: string,
): Promise<void> {
  unwrap(await commands.relinkComment(commentId, quote, selectorsJson));
}

export async function deleteComment(commentId: string): Promise<void> {
  unwrap(await commands.deleteComment(commentId));
}

/// Subscribe to comment changes. The callback receives the affected
/// `{ targetKind, targetId }`; pass a filter to scope to one target.
export function subscribeCommentEvents(
  onChange: (target: { targetKind: string; targetId: string }) => void,
  filter?: { targetKind?: string; targetId?: string },
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "commentsChanged") return;
    const targetKind = typeof event.targetKind === "string" ? event.targetKind : "";
    const targetId = typeof event.targetId === "string" ? event.targetId : "";
    if (filter?.targetKind !== undefined && filter.targetKind !== targetKind) return;
    if (filter?.targetId !== undefined && filter.targetId !== targetId) return;
    onChange({ targetKind, targetId });
  });
}

export async function searchWikiPages(
  _streamId: string,
  query: string,
  limit?: number,
): Promise<WikiPageSearchHit[]> {
  return unwrap(
    await commands.searchWikiTitles(query, limit ?? 50),
  ) as unknown as WikiPageSearchHit[];
}

export async function recordUsage(input: {
  kind: string;
  key: string | number;
  event?: string;
  streamId?: string | null;
  threadId?: string | null;
}): Promise<void> {
  unwrap(await commands.recordUsage(input.kind, JSON.stringify(input)));
}

export async function listRecentUsage(input: {
  kind: string;
  streamId?: string | null;
  threadId?: string | null;
  limit?: number;
  since?: string;
}): Promise<UsageRollup[]> {
  return unwrap(
    await commands.listRecentUsageRollup(
      input.kind,
      input.streamId ?? null,
      input.limit ?? 50,
    ),
  );
}

// `list_frequent_usage` on the Rust side currently returns PageVisit
// rows (count-ordered page-visit aggregates), not usage-event rollups
// — different table, different shape. No renderer code calls this
// helper today; keep the surface but route it through the same rollup
// endpoint as listRecentUsage so the type matches what the existing
// callers expect when one shows up. Order-by-count would need a
// dedicated `list_frequent_usage_rollup` Rust command; not building
// that until there's a caller to motivate it.
export async function listFrequentUsage(input: {
  kind: string;
  streamId?: string | null;
  threadId?: string | null;
  limit?: number;
  since?: string;
}): Promise<UsageRollup[]> {
  return unwrap(
    await commands.listRecentUsageRollup(
      input.kind,
      input.streamId ?? null,
      input.limit ?? 50,
    ),
  );
}

export type CodeQualityTool = import("./api-types.js").CodeQualityTool;
export type CodeQualityScope = import("./api-types.js").CodeQualityScope;
export type CodeQualityScanStatus = import("./api-types.js").CodeQualityScanStatus;
export type CodeQualityFindingKind = import("./api-types.js").CodeQualityFindingKind;
export type CodeQualityScanRow = import("./api-types.js").CodeQualityScanRow;
export type CodeQualityFindingRow = import("./api-types.js").CodeQualityFindingRow;

export async function listCodeQualityFindings(input: {
  streamId: string;
  tool?: CodeQualityTool;
  paths?: string[];
  scanId?: number;
}): Promise<CodeQualityFindingRow[]> {
  const raw = unwrap(
    await commands.listCodeQualityFindings(input.scanId ?? 0),
  );
  return raw.map((r) => ({
    id: r.id,
    scanId: r.scan_id,
    path: r.path,
    startLine: r.start_line,
    endLine: r.end_line,
    kind: r.kind as CodeQualityFindingKind,
    metricValue: r.metric_value,
    extra: r.extra_json ? safeParseJsonObject(r.extra_json) : null,
  }));
}

function safeParseJsonObject(s: string): Record<string, unknown> | null {
  try {
    const v = JSON.parse(s);
    return v && typeof v === "object" && !Array.isArray(v)
      ? (v as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

export function subscribeCodeQualityEvents(
  streamId: string,
  fn: (event: { scanId: number; tool: CodeQualityTool; scope: CodeQualityScope; status: CodeQualityScanStatus }) => void,
): () => void {
  // Backend emits `codeQualityScanned` { streamId, scanId, tool, scope,
  // phase: "started" | "completed" | "failed" }. Map phase → status
  // for the renderer's enum.
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "codeQualityScanned") return;
    if (event.streamId != null && event.streamId !== streamId) return;
    const phase = event.phase as string;
    const status: CodeQualityScanStatus =
      phase === "completed" ? "done" : phase === "failed" ? "failed" : "running";
    fn({
      scanId: event.scanId as number,
      tool: event.tool as CodeQualityTool,
      scope: event.scope as CodeQualityScope,
      status,
    });
  });
}

export async function getTask(id: string): Promise<Task | null> {
  return unwrap(await commands.getTask(id)) as unknown as Task | null;
}

export async function getTaskSummaries(ids: string[]): Promise<Array<{
  id: string;
  title: string;
  status: import("./api-types.js").TaskStatus;
  thread_id: string | null;
}>> {
  if (ids.length === 0) return [];
  const items = await Promise.all(
    ids.map(async (id) => {
      try {
        return unwrap(await commands.getTask(id)) as unknown as Task | null;
      } catch {
        return null;
      }
    }),
  );
  return items
    .filter((x): x is Task => x !== null)
    .map((w) => ({
      id: w.id,
      title: w.title,
      status: w.status,
      thread_id: w.thread_id,
    }));
}

/**
 * Subscribe to `usage.recorded` events. Optionally filter by `kind` so a
 * Wiki-pane consumer only refetches on wiki visits.
 */
export function subscribeUsageEvents(
  onEvent: (e: { kind: string; key: string; streamId: string | null; threadId: string | null }) => void,
  filter?: { kind?: string },
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "usageRecorded") return;
    const usageKind = event.usageKind as string;
    if (filter?.kind && usageKind !== filter.kind) return;
    onEvent({
      kind: usageKind,
      key: (event.key as string | undefined) ?? "",
      streamId: (event.streamId as string | null | undefined) ?? null,
      threadId: (event.threadId as string | null | undefined) ?? null,
    });
  });
}

export async function reorderThreadQueue(
  streamId: string,
  _threadId: string,
  entries: Array<{ id: string }>,
): Promise<void> {
  unwrap(
    await commands.reorderThreadQueue({
      streamId,
      order: entries.map((e) => e.id),
    }),
  );
}

export async function removeFollowup(_threadId: string, id: string): Promise<void> {
  unwrap(await commands.removeFollowup(id));
}

export type BackgroundTask = import("./api-types.js").BackgroundTask;

export async function listBackgroundTasks(): Promise<BackgroundTask[]> {
  return (unwrap(await commands.listBackgroundTasks()) as unknown[]).map(
    adaptBackgroundTask,
  ) as BackgroundTask[];
}

export async function getBackgroundTask(id: string): Promise<BackgroundTask | null> {
  return adaptBackgroundTask(unwrap(await commands.getBackgroundTask(id))) as
    | BackgroundTask
    | null;
}

export function subscribeBackgroundTaskEvents(
  onChange: () => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind === "backgroundTasksChanged") onChange();
  });
}

/**
 * Subscribe to changes for a single background task. The callback
 * receives the change kind ("started" | "updated" | "ended"). Use this
 * to drive in-flight UI off a kickoff IPC's returned `taskId`.
 */
export function subscribeBackgroundTask(
  taskId: string,
  onChange: (kind: "started" | "updated" | "ended") => void,
): () => void {
  // The backend `BackgroundTasksChanged` event is coarse — no taskId
  // or kind in the payload. Refetch the row on each tick and decide
  // "updated" vs "ended" from its terminal status; emit "ended" once
  // and stop, otherwise "updated". The "started" edge is whatever
  // first observation the caller sees.
  let ended = false;
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "backgroundTasksChanged") return;
    if (ended) return;
    void getBackgroundTask(taskId).then((task) => {
      if (!task) return;
      const terminal = task.status === "done" || task.status === "failed";
      if (terminal) {
        ended = true;
        onChange("ended");
      } else {
        onChange("updated");
      }
    });
  });
}

/**
 * Resolve when a background task ends (done or failed). Reads the final
 * task row so callers can inspect `task.status`, `task.error`, and
 * `task.result`. Returns null if the task disappeared (evicted) before
 * we could read it.
 */
export function awaitBackgroundTask(taskId: string): Promise<BackgroundTask | null> {
  return new Promise((resolve) => {
    let settled = false;
    const finish = async () => {
      if (settled) return;
      settled = true;
      unsubscribe();
      resolve(await getBackgroundTask(taskId));
    };
    const unsubscribe = subscribeBackgroundTask(taskId, (kind) => {
      if (kind === "ended") void finish();
    });
    // Race condition: the task may have already ended before we
    // subscribed. Check the current row once on entry.
    void getBackgroundTask(taskId).then((task) => {
      if (task && (task.status === "done" || task.status === "failed")) void finish();
    });
  });
}

export async function listAllRefs(_streamId: string): Promise<import("./api-types.js").RefOption[]> {
  return listGitRefs() as unknown as Promise<
    import("./api-types.js").RefOption[]
  >;
}

export async function listTaskEvents(
  _streamId: string,
  _threadId: string,
  itemId?: string,
): Promise<TaskEvent[]> {
  return unwrap(
    await commands.listTaskEvents(itemId ?? null, null),
  ) as unknown as TaskEvent[];
}

export async function getBranchChanges(
  streamId: string,
  baseRef?: string,
): Promise<import("./api-types.js").BranchChanges & { resolvedBaseRef: string | null }> {
  // Resolve the base ref if not given, by reading the change scopes.
  const resolved = baseRef ?? (await getChangeScopes(streamId)).branchBase ?? "main";
  // The Rust binding emits `change: ChangeKind`; renderer call sites
  // (App.tsx uncommittedSummary, ProjectPanel scopedDeletions,
  // CommitDetailSlideover, UncommittedChangesPage, GitDashboardPage)
  // read `entry.status`. Translate here — without this the Uncommitted
  // rail section silently hides because every f.status is undefined.
  const raw = unwrap(await commands.getBranchChanges(streamId, resolved));
  const files = raw.files.map((entry) => ({
    path: entry.path,
    status: entry.change as import("./api-types.js").GitFileStatus,
    additions: entry.additions,
    deletions: entry.deletions,
  }));
  return {
    base_ref: raw.base_ref,
    merge_base: raw.merge_base,
    files,
    resolvedBaseRef: resolved,
  };
}

export async function readFileAtRef(
  _streamId: string,
  ref: string,
  path: string,
): Promise<{ content: string | null }> {
  const content = unwrap(await commands.readFileAtRef(ref, path));
  return { content };
}

export async function listTaskEfforts(itemId: string): Promise<EffortDetail[]> {
  // The Tauri command returns flat `TaskEffort` rows. Consumers
  // (TaskPage activity timeline, useBacklinks, TaskDetail) expect
  // the richer `EffortDetail` shape with changed paths + counts.
  // Pull the per-effort `task_effort_file` rows in parallel —
  // that's the canonical authorship list (what the agent declared
  // via `complete_task` / `amend_effort`). start_snapshot and
  // end_snapshot are still null until a "snapshot by id" IPC
  // exists; consumers tolerate that.
  const rows = unwrap(await commands.listTaskEfforts(itemId)) as unknown as TaskEffort[];
  const filesByEffort = await Promise.all(
    rows.map(async (effort) => {
      try {
        const files = await listEffortFiles(effort.id);
        return [effort.id, files] as const;
      } catch {
        return [effort.id, [] as Array<{ path: string; change: "created" | "updated" | "deleted" }>] as const;
      }
    }),
  );
  const filesById = new Map(filesByEffort);
  return rows.map((rawEffort) => {
    const files = filesById.get(rawEffort.id) ?? [];
    const counts = { created: 0, updated: 0, deleted: 0 };
    for (const f of files) counts[f.change]++;
    // The binding types snapshot ids as numbers; normalize to the
    // app's string contract so they survive the trip into
    // `TreeVersion` (Rust expects a string `id`) — a raw number
    // surfaces as an opaque "ipc error" when opening an effort diff.
    const effort: TaskEffort = {
      ...rawEffort,
      start_snapshot_id: normalizeSnapshotId(rawEffort.start_snapshot_id),
      end_snapshot_id: normalizeSnapshotId(rawEffort.end_snapshot_id),
    };
    return {
      effort,
      start_snapshot: null,
      end_snapshot: null,
      changed_paths: files.map((f) => f.path),
      counts,
    };
  });
}

export type { EffortObservation } from "./tauri-bridge/index.js";
export type { AgentNudge } from "./tauri-bridge/index.js";

/** Collection observations (test-run / diff-coverage) for an effort,
 *  newest-first. Optional `kind` filter. */
export async function listEffortObservations(
  effortId: string,
  kind?: string,
): Promise<import("./tauri-bridge/index.js").EffortObservation[]> {
  return unwrap(
    await commands.listEffortObservations(effortId, kind ?? null),
  ) as unknown as import("./tauri-bridge/index.js").EffortObservation[];
}

export type { EffortMetricDelta } from "./tauri-bridge/index.js";

/** Per-metric roll-up over an effort — grouped before→after deltas for the
 *  task/effort page's metrics panel (attributed per family; tsk250). */
export async function listEffortMetricDeltas(
  effortId: string,
): Promise<import("./tauri-bridge/index.js").EffortMetricDelta[]> {
  return unwrap(
    await commands.listEffortMetricDeltas(effortId),
  ) as unknown as import("./tauri-bridge/index.js").EffortMetricDelta[];
}

export type {
  MetricSpec,
  SeriesPoint,
  FactFinding,
  MetricCatalogEntry,
  RollupRow,
} from "./tauri-bridge/index.js";

/** The metric catalog — every known metric SPEC (built-in / global / project).
 *  A metric is an aggregation defined OVER a measure (epic tsk12), not a second
 *  store of rows. Optional `language` / `scope` filter. */
export async function listMetricDefinitions(
  language?: string,
  scope?: string,
): Promise<import("./tauri-bridge/index.js").MetricSpec[]> {
  return unwrap(
    await commands.listMetricDefinitions(language ?? null, scope ?? null),
  ) as unknown as import("./tauri-bridge/index.js").MetricSpec[];
}

/** Time series for one metric (by spec `key`), newest-first — one point per
 *  capture aggregated over the metric's source-measure facts (epic tsk12).
 *  `groupBy` slices by a conformed dimension (`subject` / `branch` /
 *  `oxplow.model` / …), yielding one series-point per (capture × group). */
export async function listMetricSamples(
  metricKey: string,
  limit?: number,
  groupBy?: string | null,
): Promise<import("./tauri-bridge/index.js").SeriesPoint[]> {
  return unwrap(
    await commands.listMetricSamples(metricKey, limit ?? null, groupBy ?? null),
  ) as unknown as import("./tauri-bridge/index.js").SeriesPoint[];
}

/** Roll up a metric (by spec `key`) by a dimension — `"package"` (the file's
 *  parent directory), a conformed dim (`oxplow.severity`), a `subject` roll-up,
 *  or any `dims_json` key — latest value per subject summed per dimension
 *  value, largest first. The Metric Detail Breakdown + subject breakdown
 *  (tsk328 package / tsk319 language). */
export async function metricDimensionRollup(
  metricKey: string,
  dimension: string,
): Promise<import("./tauri-bridge/index.js").RollupRow[]> {
  return unwrap(
    await commands.metricDimensionRollup(metricKey, dimension),
  ) as unknown as import("./tauri-bridge/index.js").RollupRow[];
}

/** The located items behind one metric (by spec `key`) — the read-time finding
 *  view over its filtered facts (epic tsk12). `captureId` scopes to one
 *  recording's drill-in (findings table / per-file coverage / per-case tests);
 *  omit for every matching fact. */
export async function listMetricFindings(
  metricKey: string,
  captureId?: number | null,
): Promise<import("./tauri-bridge/index.js").FactFinding[]> {
  return unwrap(
    await commands.listMetricFindings(metricKey, captureId ?? null),
  ) as unknown as import("./tauri-bridge/index.js").FactFinding[];
}

/** The available metric catalog (built-in ∪ global ∪ project) + each entry's
 *  enabled-in-this-project flag. Drives the Catalog page (tsk219). */
export async function listMetricCatalog(): Promise<
  import("./tauri-bridge/index.js").MetricCatalogEntry[]
> {
  return unwrap(
    await commands.listMetricCatalog(),
  ) as unknown as import("./tauri-bridge/index.js").MetricCatalogEntry[];
}

/** Enable (add a `use:`) or disable (remove) a metric in `.oxplow/project.yaml`. */
export async function setMetricEnabled(key: string, enabled: boolean): Promise<void> {
  unwrap(await commands.setMetricEnabled(key, enabled));
}

/** Set a metric's `target` override in `.oxplow/project.yaml` (the Catalog inline edit,
 *  tsk233). `null` clears that override. `trigger` is inherent to the
 *  definition and not overridable (tsk290). */
export async function setMetricOverride(
  key: string,
  target: number | null,
): Promise<void> {
  unwrap(await commands.setMetricOverride(key, target));
}

/** Scaffold a new project gauge metric — writes a starter script + a `metrics:`
 *  entry into `.oxplow/project.yaml`; returns the project-relative script path to open
 *  (the Catalog "New metric" action, tsk234). */
export async function scaffoldMetric(
  key: string,
  title: string | null,
  language: string | null,
  glob: string | null,
  scope: "project" | "global" | null = null,
): Promise<string> {
  return unwrap(await commands.scaffoldMetric(key, title, language, glob, scope));
}

/** Efforts whose span overlaps `[windowStart, windowEnd]` (RFC-3339) — the
 *  Metrics Explorer's effort-band overlay (tsk233). */
export async function listEffortsInWindow(
  windowStart: string,
  windowEnd: string,
): Promise<TaskEffort[]> {
  return unwrap(
    await commands.listEffortsInWindow(windowStart, windowEnd),
  ) as unknown as TaskEffort[];
}

/** Persisted agent nudges (report-less-run / commit-hygiene) fired for an
 *  effort, newest-first. Drives the collapsed "Agent nudges" debug sub-view. */
export async function listNudgesForEffort(
  effortId: string,
): Promise<import("./tauri-bridge/index.js").AgentNudge[]> {
  return unwrap(
    await commands.listNudgesForEffort(effortId),
  ) as unknown as import("./tauri-bridge/index.js").AgentNudge[];
}

export type { AgentTokenUsage, TokenUsageTotals } from "./tauri-bridge/index.js";

/** Per-turn agent token-usage rows for an effort, newest-first (tsk104). */
export async function listTokenUsageForEffort(
  effortId: string,
): Promise<import("./tauri-bridge/index.js").AgentTokenUsage[]> {
  return unwrap(
    await commands.listTokenUsageForEffort(effortId),
  ) as unknown as import("./tauri-bridge/index.js").AgentTokenUsage[];
}

/** Summed token totals for one effort. */
export async function getEffortTokenTotals(
  effortId: string,
): Promise<import("./tauri-bridge/index.js").TokenUsageTotals> {
  return unwrap(
    await commands.getEffortTokenTotals(effortId),
  ) as unknown as import("./tauri-bridge/index.js").TokenUsageTotals;
}

/** Summed token totals for a whole thread (the Work panel running total). */
export async function getThreadTokenTotals(
  threadId: string,
): Promise<import("./tauri-bridge/index.js").TokenUsageTotals> {
  return unwrap(
    await commands.getThreadTokenTotals(threadId),
  ) as unknown as import("./tauri-bridge/index.js").TokenUsageTotals;
}

export type {
  AgentKindTokenUsage,
  ModelTokenUsage,
  TokenUsageByDay,
} from "./tauri-bridge/index.js";

/** Summed token totals across every recorded turn (Token Analytics). */
export async function getTokenTotalsOverall(): Promise<
  import("./tauri-bridge/index.js").TokenUsageTotals
> {
  return unwrap(
    await commands.tokenTotalsOverall(),
  ) as unknown as import("./tauri-bridge/index.js").TokenUsageTotals;
}

/** Token totals grouped by agent/harness, busiest first. */
export async function tokenUsageByAgent(): Promise<
  import("./tauri-bridge/index.js").AgentKindTokenUsage[]
> {
  return unwrap(
    await commands.tokenUsageByAgent(),
  ) as unknown as import("./tauri-bridge/index.js").AgentKindTokenUsage[];
}

/** Token totals grouped by (agent_kind, model), busiest first. */
export async function tokenUsageByModel(): Promise<
  import("./tauri-bridge/index.js").ModelTokenUsage[]
> {
  return unwrap(
    await commands.tokenUsageByModel(),
  ) as unknown as import("./tauri-bridge/index.js").ModelTokenUsage[];
}

/** Token volume bucketed by day over the last `days` days (trend chart). */
export async function tokenUsageByDay(
  days: number,
): Promise<import("./tauri-bridge/index.js").TokenUsageByDay[]> {
  return unwrap(
    await commands.tokenUsageByDay(days),
  ) as unknown as import("./tauri-bridge/index.js").TokenUsageByDay[];
}

export async function listFileSnapshots(
  streamId: string,
  limit?: number,
): Promise<FileSnapshot[]> {
  return unwrap(
    await commands.listFileSnapshotsForStream(streamId, limit ?? null),
  ) as unknown as FileSnapshot[];
}

/** Snapshot row — one per `request_snapshot()` call that captured
 *  anything. Local History dashboard surfaces this list. */
export interface Snapshot {
  id: number;
  streamId: string;
  createdAt: string;
  fileCount: number;
  gitCommit: string | null;
  /** Short name of the branch HEAD was on at capture; null for pre-V42
   *  rows, a detached HEAD, or a non-git directory. */
  gitBranch: string | null;
}

/** Per-file snapshot history — every `file_snapshot` row for this
 *  path across every snapshot, newest first. Drives the per-file
 *  history surface on FilePage. */
export interface FileSnapshotRow {
  id: number;
  streamId: string;
  path: string;
  blobHash: string | null;
  sizeBytes: number;
  capturedAt: string;
  oversize: boolean;
  snapshotId: number | null;
  mtimeMs: number | null;
}

export async function listFileSnapshotsForPath(
  path: string,
): Promise<FileSnapshotRow[]> {
  const rows = unwrap(await commands.listSnapshots(path)) as unknown as Array<{
    id: number;
    stream_id: string;
    path: string;
    blob_hash: string | null;
    size_bytes: number;
    captured_at: string;
    storage: "oxplow" | "git" | "oversize" | "deleted";
    snapshot_id: number | null;
    mtime_ms: number | null;
  }>;
  return rows.map((r) => ({
    id: r.id,
    streamId: r.stream_id,
    path: r.path,
    blobHash: r.blob_hash,
    sizeBytes: r.size_bytes,
    capturedAt: r.captured_at,
    oversize: r.storage === "oversize",
    snapshotId: r.snapshot_id,
    mtimeMs: r.mtime_ms,
  }));
}

/** List snapshot rows for a stream, newest first. */
export async function listSnapshots(streamId: string, limit?: number): Promise<Snapshot[]> {
  const rows = unwrap(
    await commands.listSnapshotsForStream(streamId, limit ?? null),
  ) as unknown as Array<{
    id: number;
    stream_id: string;
    created_at: string;
    file_count: number;
    git_commit: string | null;
    git_branch: string | null;
  }>;
  return rows.map((r) => ({
    id: r.id,
    streamId: r.stream_id,
    createdAt: r.created_at,
    fileCount: r.file_count,
    gitCommit: r.git_commit,
    gitBranch: r.git_branch,
  }));
}

/** Aggregate created/modified/deleted counts for a snapshot. */
export async function getSnapshotStats(snapshotId: number): Promise<{
  created: number;
  modified: number;
  deleted: number;
  total: number;
}> {
  const raw = unwrap(await commands.getSnapshotStats(snapshotId)) as unknown as {
    created: number;
    modified: number;
    deleted: number;
    total: number;
  };
  return raw;
}

/** Total on-disk size of the content-addressed blob store. */
export async function getBlobStorageBytes(): Promise<number> {
  return unwrap(await commands.getBlobStorageBytes()) as unknown as number;
}

/** Detail-page file row for a snapshot — one per captured file. */
export interface SnapshotFile {
  id: number;
  path: string;
  blobHash: string | null;
  sizeBytes: number;
  oversize: boolean;
  mtimeMs: number | null;
}

/** Every captured file for one snapshot. */
export async function listFilesForSnapshot(snapshotId: number): Promise<SnapshotFile[]> {
  const rows = unwrap(
    await commands.listFilesForSnapshot(snapshotId),
  ) as unknown as Array<{
    id: number;
    path: string;
    blob_hash: string | null;
    size_bytes: number;
    storage: "oxplow" | "git" | "oversize" | "deleted";
    mtime_ms: number | null;
  }>;
  return rows.map((r) => ({
    id: r.id,
    path: r.path,
    blobHash: r.blob_hash,
    sizeBytes: r.size_bytes,
    oversize: r.storage === "oversize",
    mtimeMs: r.mtime_ms,
  }));
}

export async function getSnapshotSummary(
  snapshotId: string,
  _previousSnapshotId?: string | null,
): Promise<SnapshotSummary | null> {
  const id = Number(snapshotId);
  if (!Number.isFinite(id)) return null;
  return unwrap(await commands.getSnapshotSummary(id)) as unknown as SnapshotSummary | null;
}

export async function getSnapshotPairDiff(
  beforeSnapshotId: string | null,
  afterSnapshotId: string,
  _path: string,
): Promise<SnapshotDiffResult> {
  return unwrap(
    await commands.getSnapshotPairDiff(
      beforeSnapshotId === null ? null : Number(beforeSnapshotId),
      Number(afterSnapshotId),
    ),
  ) as unknown as SnapshotDiffResult;
}

/** Diff two endpoints, each a snapshot id or a git commit. `start =
 *  null` diffs `end` against the empty tree (everything added). Powers
 *  the effort / local-history diff view. Mixed snapshot/commit and
 *  working-tree endpoints are not yet supported by the backend. */
export async function diffEndpoints(
  start: DiffEndpoint | null,
  end: DiffEndpoint,
): Promise<DiffEntry[]> {
  return unwrap(await commands.diffEndpoints(start, end));
}

/** Endpoint constructors so call sites read as intent. */
export const snapshotEndpoint = (snapshotId: number): DiffEndpoint => ({
  kind: "snapshot",
  snapshot_id: snapshotId,
});
export const commitEndpoint = (sha: string): DiffEndpoint => ({ kind: "commit", sha });

export async function getEffortFiles(effortId: string): Promise<SnapshotSummary | null> {
  return unwrap(
    await commands.getEffortFiles(effortId),
  ) as unknown as SnapshotSummary | null;
}

/** One effort by id — its snapshot bracket + task id, so the diff view
 *  can resolve `effortDiffRef(effortId)` into (start, end) endpoints.
 *  `null` when the id is unknown. */
export async function getEffort(effortId: string): Promise<OverlappingEffort | null> {
  const row = unwrap(await commands.getEffort(effortId)) as unknown as {
    id: string;
    task_id: string;
    thread_id: string;
    started_at: string;
    ended_at: string | null;
    start_snapshot_id: number | null;
    end_snapshot_id: number | null;
    summary: string | null;
  } | null;
  if (!row) return null;
  return {
    effortId: row.id,
    taskId: row.task_id,
    threadId: row.thread_id,
    startedAt: row.started_at,
    endedAt: row.ended_at,
    startSnapshotId: row.start_snapshot_id,
    endSnapshotId: row.end_snapshot_id,
    summary: row.summary,
  };
}

/** Per-effort touched_files list — the canonical authorship list
 *  (LLM-declared via `complete_task` + any subsequent `amend_effort`
 *  corrections). Distinct from getEffortFiles, which had a misleading
 *  return-type annotation; this is the well-typed wrapper. */
export async function listEffortFiles(
  effortId: string,
): Promise<Array<{ path: string; change: "created" | "updated" | "deleted" }>> {
  const rows = unwrap(await commands.getEffortFiles(effortId)) as unknown as Array<{
    path: string;
    change: "created" | "updated" | "deleted";
  }>;
  return rows.map((r) => ({ path: r.path, change: r.change }));
}

/** One (snapshot, effort) pair. `completedHere` is true when the
 *  effort ended exactly at this snapshot; otherwise the effort was
 *  in flight at this snapshot. Callers group by `snapshotId` and
 *  resolve task titles via `getTaskSummaries` (the IPC only carries
 *  effort columns). */
export interface EffortAtSnapshot {
  snapshotId: number;
  effortId: string;
  tasksId: string;
  threadId: string;
  startSnapshotId: number | null;
  endSnapshotId: number | null;
  completedHere: boolean;
}

/** For each snapshot id in the input list, the wiki slugs whose
 *  body changed in that snapshot. Drives wiki badges on the
 *  Local History dashboard. */
export async function listWikiSlugsForSnapshots(
  snapshotIds: number[],
): Promise<Array<{ snapshotId: number; slug: string }>> {
  const rows = unwrap(
    await commands.listWikiSlugsForSnapshots(snapshotIds),
  ) as unknown as Array<[number, string]>;
  return rows.map(([snapshotId, slug]) => ({ snapshotId, slug }));
}

export type { EffortChangedPaths } from "./tauri-bridge/index.js";

/** Snapshot-bracket changed paths for an effort, split into the paths the
 *  effort CLAIMED (task_effort_file) vs the `unclaimed` rest (parallel/
 *  external writes, formatters, capture gaps) — the claim-aware view that
 *  matches the history grouping. Both empty when the effort has no
 *  start/end snapshot pin yet. */
export async function listChangedPathsForEffort(
  effortId: string,
): Promise<import("./tauri-bridge/index.js").EffortChangedPaths> {
  return unwrap(
    await commands.listChangedPathsForEffort(effortId),
  ) as unknown as import("./tauri-bridge/index.js").EffortChangedPaths;
}

export async function listEffortsAtSnapshots(
  snapshotIds: number[],
): Promise<EffortAtSnapshot[]> {
  const rows = unwrap(
    await commands.listEffortsAtSnapshots(snapshotIds),
  ) as unknown as Array<{
    snapshot_id: number;
    effort: {
      id: string;
      task_id: string;
      thread_id: string;
      start_snapshot_id: number | null;
      end_snapshot_id: number | null;
    };
  }>;
  return rows.map((r) => ({
    snapshotId: r.snapshot_id,
    effortId: r.effort.id,
    tasksId: r.effort.task_id,
    threadId: r.effort.thread_id,
    startSnapshotId: r.effort.start_snapshot_id,
    endSnapshotId: r.effort.end_snapshot_id,
    completedHere: r.effort.end_snapshot_id === r.snapshot_id,
  }));
}

export interface OverlappingEffort {
  effortId: string;
  taskId: string;
  threadId: string;
  startedAt: string;
  endedAt: string | null;
  startSnapshotId: number | null;
  endSnapshotId: number | null;
  summary: string | null;
}

/** Efforts whose snapshot window overlaps the half-open range
 *  `(rangeStart, rangeEnd]` — including ones that merely started or
 *  ended inside it, contain it, or are still open. Drives the diff
 *  view's roster of other efforts that overlapped the diffed range. */
export async function listEffortsOverlappingRange(
  rangeStart: number,
  rangeEnd: number,
): Promise<OverlappingEffort[]> {
  const rows = unwrap(
    await commands.listEffortsOverlappingRange(rangeStart, rangeEnd),
  ) as unknown as Array<{
    id: string;
    task_id: string;
    thread_id: string;
    started_at: string;
    ended_at: string | null;
    start_snapshot_id: number | null;
    end_snapshot_id: number | null;
    summary: string | null;
  }>;
  return rows.map((r) => ({
    effortId: r.id,
    taskId: r.task_id,
    threadId: r.thread_id,
    startedAt: r.started_at,
    endedAt: r.ended_at,
    startSnapshotId: r.start_snapshot_id,
    endSnapshotId: r.end_snapshot_id,
    summary: r.summary,
  }));
}

export async function restoreFileFromSnapshot(
  _streamId: string,
  snapshotId: string,
  _path: string,
): Promise<void> {
  unwrap(await commands.restoreFileFromSnapshot(Number(snapshotId)));
}

export interface FileSnapshotCreatedEventPayload {
  streamId: string;
  snapshotId: string;
  kind: SnapshotSource;
  effortId: string | null;
  threadId: string | null;
}

export function subscribeSnapshotEvents(
  streamId: string,
  fn: (payload: FileSnapshotCreatedEventPayload) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "fileSnapshotCreated" && event.kind !== "fileSnapshotsBatchCreated") return;
    const eventStreamId = (event.streamId as string | null | undefined) ?? null;
    if (eventStreamId != null && eventStreamId !== streamId) return;
    fn({
      streamId: eventStreamId ?? streamId,
      snapshotId: String(event.snapshotId),
      kind: (event.source as SnapshotSource) ?? "effort-event",
      effortId: (event.effortId as string | null | undefined) ?? null,
      threadId: (event.threadId as string | null | undefined) ?? null,
    });
  });
}

export async function listWorkspaceEntries(streamId: string, path = ""): Promise<WorkspaceEntry[]> {
  return unwrap(
    await commands.listWorkspaceEntries(streamId || null, path),
  ) as unknown as WorkspaceEntry[];
}

export async function listWorkspaceFiles(streamId: string): Promise<{
  files: WorkspaceIndexedFile[];
  summary: WorkspaceStatusSummary;
}> {
  const [filesRes, summary] = await Promise.all([
    commands.listWorkspaceFiles(streamId || null),
    getWorkspaceStatusSummary(streamId),
  ]);
  // The wire row is snake_case (`git_status`); map it onto our camelCase
  // `WorkspaceIndexedFile`. Without this, `f.gitStatus` is always
  // `undefined`, so `f.gitStatus !== null` matched every file and the
  // Uncommitted filter showed the whole tree (tsk211).
  const rows = unwrap(filesRes) as unknown as Array<{ path: string; git_status: GitFileStatus | null }>;
  const files: WorkspaceIndexedFile[] = rows.map((r) => ({ path: r.path, gitStatus: r.git_status }));
  return { files, summary };
}

export async function readWorkspaceFile(streamId: string, path: string): Promise<WorkspaceFile> {
  return unwrap(await commands.readWorkspaceFile(streamId || null, path));
}

/**
 * Versioned file read. Routes to the working tree, a git ref, or
 * (eventually) a local-history snapshot based on `version`. The
 * single-chokepoint replacement for the historical pair
 * `readWorkspaceFile` + `readFileAtRef` — new code must use this so
 * "what version are we reading?" is a typed answer at every call
 * site. Returns `null` when the path doesn't exist at that version.
 */
export async function readFile(
  streamId: string,
  path: string,
  version: import("./file-version.js").FileVersion,
): Promise<string | null> {
  return unwrap(await commands.readFile(streamId || null, path, version));
}

export async function getWorkspaceStatusSummary(
  streamId: string,
): Promise<WorkspaceStatusSummary> {
  return unwrap(
    await commands.getWorkspaceStatusSummary(streamId || null),
  ) as unknown as WorkspaceStatusSummary;
}

export async function writeWorkspaceFile(
  streamId: string,
  path: string,
  content: string,
): Promise<WorkspaceFile> {
  return unwrap(await commands.writeWorkspaceFile(streamId || null, path, content));
}

export async function createWorkspaceFile(
  streamId: string,
  path: string,
  content = "",
): Promise<WorkspaceFile> {
  return unwrap(await commands.createWorkspaceFile(streamId || null, path, content));
}

export async function createWorkspaceDirectory(
  streamId: string,
  path: string,
): Promise<WorkspacePathChange> {
  unwrap(await commands.createWorkspaceDirectory(streamId || null, path));
  return { path };
}

export async function renameWorkspacePath(
  streamId: string,
  fromPath: string,
  toPath: string,
): Promise<WorkspaceRenameResult> {
  unwrap(await commands.renameWorkspacePath(streamId || null, fromPath, toPath));
  return { fromPath, toPath };
}

export async function deleteWorkspacePath(
  streamId: string,
  path: string,
): Promise<WorkspacePathChange> {
  unwrap(await commands.deleteWorkspacePath(streamId || null, path));
  return { path };
}

export function subscribeOxplowEvents(
  listener: (event: OxplowEvent) => void,
): () => void {
  let stopped = false;
  const unlistenPromise = listen(EVENT_CHANNELS.oxplow, (e) => {
    if (stopped) return;
    listener(e.payload as OxplowEvent);
  });
  return () => {
    stopped = true;
    void unlistenPromise.then((u) => u());
  };
}

export function subscribeWorkspaceContext(
  onEvent: (next: WorkspaceContext) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "workspaceContextChanged") return;
    onEvent({ gitEnabled: Boolean(event.gitEnabled) });
  });
}

export function subscribeWorkspaceEvents(
  streamId: string,
  onEvent: (event: WorkspaceWatchEvent) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "workspaceChanged") return;
    if (event.streamId !== streamId) return;
    onEvent({
      id: 0,
      streamId,
      kind: event.changeKind as WorkspaceWatchEvent["kind"],
      path: event.path as string,
      t: Date.now(),
    });
  });
}

export function subscribeGitRefsEvents(
  streamId: string,
  onEvent: () => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "gitRefsChanged") return;
    if (event.streamId !== streamId) return;
    onEvent();
  });
}

export type tasksChangeKind = "created" | "updated" | "note" | "linked" | "deleted" | "reordered" | "moved";

export interface tasksChangeEvent {
  streamId: string;
  threadId: string;
  kind: tasksChangeKind;
  itemId: number | null;
}

export type AgentStatus = "working" | "waiting" | "stalled";

export interface AgentStatusEntry {
  streamId: string;
  threadId: string;
  status: AgentStatus;
}

/// Collapse the backend `AgentStatusState` enum to the dot's alphabet.
/// "running" → working; "stalled" (derived when a Running hook log
/// goes silent past the stall threshold — the agent died without ever
/// emitting a Stop hook) stays distinct so the dot can render it as a
/// failure rather than ordinary waiting; everything else (idle /
/// awaiting_user / stopped / error) → waiting.
export function collapseAgentStatusState(raw: string | undefined): AgentStatus {
  if (raw === "running") return "working";
  if (raw === "stalled") return "stalled";
  return "waiting";
}

/// Synthesize an Interrupt hook for `threadId`. Used by the agent
/// terminal's Escape handler — Claude Code cancels the in-flight turn
/// on Escape but does not emit a Stop/Interrupt hook itself, so the
/// working-dot would stay Running until the next user prompt.
/// Posting an Interrupt envelope here closes any open agent_turn and
/// flips the derived status back to Idle immediately.
export async function recordUserInterrupt(threadId: string, streamId: string | null): Promise<void> {
  unwrap(
    await commands.ingestHookEvent({
      kind: "interrupt",
      thread_id: threadId,
      stream_id: streamId,
      session_id: null,
      payload_json: JSON.stringify({ source: "user-escape" }),
      prompt: null,
    }),
  );
}

export async function listAgentStatuses(_streamId?: string): Promise<AgentStatusEntry[]> {
  // The Rust binding returns the raw `AgentStatus` row
  // ({ thread_id, pane_target, state: "idle"|"running"|... }). The
  // renderer only cares about the dot's narrow alphabet, so collapse
  // the AgentStatusState enum here (see collapseAgentStatusState).
  // Without this transform the consumer reads `entry.threadId` and
  // `entry.status` off raw rows that have neither field, so the dot
  // never leaves its waiting fallback.
  const rows = unwrap(await commands.listAgentStatuses());
  return rows.map((row) => ({
    streamId: "",
    threadId: row.thread_id,
    status: collapseAgentStatusState(row.state),
  }));
}

export type FinishedEntry =
  | { kind: "task"; itemId: string; title: string; t: string }
  | { kind: "wiki"; slug: string; title: string; t: string };

export async function listRecentlyFinished(threadId: string | null, limit: number): Promise<FinishedEntry[]> {
  return unwrap(await commands.listRecentlyFinished(threadId, limit)) as FinishedEntry[];
}

export async function clearRecentlyFinished(threadId: string | null): Promise<void> {
  unwrap(await commands.clearRecentlyFinished(threadId));
}

export interface PageVisitInputApi {
  refKind: string;
  refId: string;
  payload: unknown;
  label: string;
  streamId?: string | null;
  threadId?: string | null;
  source?: string | null;
}

export interface PageVisitApi {
  id: number;
  t: string;
  streamId: string | null;
  threadId: string | null;
  refKind: string;
  refId: string;
  payload: unknown;
  label: string;
  source: string | null;
}

export interface TopVisitedRowApi {
  refId: string;
  refKind: string;
  payload: unknown;
  label: string;
  count: number;
  lastT: string;
}

export interface CountByDayRowApi {
  day: string;
  count: number;
}

export async function recordPageVisit(input: PageVisitInputApi): Promise<void> {
  unwrap(
    await commands.recordPageVisit(
      input.refKind,
      input.refId,
      input.label,
      null,
      input.threadId ?? null,
    ),
  );
}

export async function listRecentPageVisits(opts: {
  threadId?: string | null;
  limit: number;
  dedupeByRef?: boolean;
  excludeKinds?: string[];
}): Promise<PageVisitApi[]> {
  // Thread filter is applied at the SQL layer; exclude/dedupe still
  // happen client-side. Over-fetch so post-filtering has enough rows.
  const raw = await unwrap(
    await commands.listRecentPageVisits(
      Math.max(opts.limit ?? 50, 50) * 4,
      opts.threadId ?? null,
    ),
  );
  const exclude = new Set(opts.excludeKinds ?? []);
  const seen = new Set<string>();
  const out: PageVisitApi[] = [];
  for (const v of raw) {
    if (exclude.has(v.page_kind)) continue;
    const key = `${v.page_kind}:${v.page_id}`;
    if (opts.dedupeByRef && seen.has(key)) continue;
    seen.add(key);
    out.push({
      id: Number(v.id),
      t: v.visited_at,
      streamId: null,
      threadId: null,
      refKind: v.page_kind,
      refId: v.page_id,
      payload: null,
      label: v.label ?? v.page_id,
      source: null,
    });
    if (out.length >= (opts.limit ?? 50)) break;
  }
  return out;
}

export async function topVisitedPages(opts: {
  threadId?: string | null;
  sinceT?: string | null;
  limit: number;
  excludeKinds?: string[];
}): Promise<TopVisitedRowApi[]> {
  const raw = await unwrap(
    await commands.topVisitedPages(
      Math.max(opts.limit ?? 50, 50) * 4,
      opts.threadId ?? null,
    ),
  );
  const exclude = new Set(opts.excludeKinds ?? []);
  const out: TopVisitedRowApi[] = [];
  for (const v of raw) {
    if (exclude.has(v.page_kind)) continue;
    out.push({
      refId: v.page_id,
      refKind: v.page_kind,
      payload: null,
      label: v.page_id, // top-visited has no per-row label; rendered consumers
                       // typically render their own derived form anyway.
      count: v.visit_count,
      lastT: "",
    });
    if (out.length >= (opts.limit ?? 50)) break;
  }
  return out;
}

export async function countPageVisitsByDay(opts: {
  refId?: string;
  threadId?: string | null;
  sinceT?: string;
  untilT?: string;
}): Promise<CountByDayRowApi[]> {
  // Bindings expose a daily count for the last N days; the Rust
  // command takes `days`, not since/until ranges. Default to 30
  // when no window is provided.
  return unwrap(await commands.countPageVisitsByDay(30)) as unknown as CountByDayRowApi[];
}

export function subscribePageVisitEvents(onEvent: () => void): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind === "pageVisitChanged") onEvent();
  });
}

/** Drop every visit row for a given page reference. Used when a page
 *  is deleted (real persistent or virtual, e.g. an op-error entry) so
 *  it disappears from rail history. Generic — not tied to any one
 *  page kind. */
export async function forgetPage(refKind: string, refId: string): Promise<void> {
  unwrap(await commands.forgetPage(refKind, refId));
}

export async function getRepoConflictState(
  streamId: string,
): Promise<import("./api-types.js").RepoConflictState> {
  return unwrap(
    await commands.getRepoConflictState(streamId),
  ) as unknown as import("./api-types.js").RepoConflictState;
}

export function subscribeAgentStatus(
  streamId: string | "all",
  onEvent: (entry: AgentStatusEntry) => void,
): () => void {
  // The backend `AgentStatusChanged` event payload carries the
  // derived state directly, so the renderer can update without a
  // refetch round-trip. Map the AgentStatusState enum to the dot's
  // alphabet the same way listAgentStatuses() does.
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "agentStatusChanged") return;
    const threadId = event.threadId as string | undefined;
    const rawState = event.state as string | undefined;
    if (!threadId || !rawState) return;
    const status = collapseAgentStatusState(rawState);
    // streamId filter is a no-op — the event doesn't carry stream
    // attribution. The single caller in App.tsx subscribes with "all".
    void streamId;
    onEvent({ streamId: "", threadId, status });
  });
}

export interface OpenAgentTurn {
  id: string;
  threadId: string;
  prompt: string;
  startedAt: string;
}

/// Open agent turns (`ended_at IS NULL`) for a thread. The Work
/// panel renders each as a live spinner row at the top of the In
/// Progress section; the Stop hook closes the row and an
/// `agentTurnsChanged` event triggers the refetch that removes it.
export async function listOpenAgentTurns(threadId: string): Promise<OpenAgentTurn[]> {
  const rows = unwrap(await commands.listOpenAgentTurns(threadId));
  return rows.map((row) => ({
    id: row.id,
    threadId: row.thread_id,
    prompt: row.prompt,
    startedAt: row.started_at,
  }));
}

/// Fires whenever an agent turn opens or closes on any thread.
export function subscribeAgentTurns(onEvent: (event: { threadId: string }) => void): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "agentTurnsChanged") return;
    const threadId = event.threadId as string | undefined;
    if (!threadId) return;
    onEvent({ threadId });
  });
}

export interface AgentStallAlertEvent {
  threadId: string;
  inProgressCount: number;
  waitingMs: number;
}

/// The backend stall watchdog fires this (once per stall episode) when
/// a thread has in_progress tasks but its agent has not been running
/// past the alert threshold — e.g. the agent process died on an API
/// error without a Stop hook, or stopped cleanly and never resumed.
export function subscribeAgentStallAlerts(onEvent: (alert: AgentStallAlertEvent) => void): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "agentStallAlert") return;
    const threadId = event.threadId as string | undefined;
    if (!threadId) return;
    onEvent({
      threadId,
      inProgressCount: (event.inProgressCount as number | undefined) ?? 0,
      waitingMs: (event.waitingMs as number | undefined) ?? 0,
    });
  });
}

/// Toast copy for a stall alert. Pure so tests can pin the wording.
export function formatAgentStallAlert(alert: AgentStallAlertEvent): string {
  const minutes = Math.max(1, Math.round(alert.waitingMs / 60_000));
  const tasks = alert.inProgressCount === 1 ? "1 in-progress task" : `${alert.inProgressCount} in-progress tasks`;
  return `Agent appears stalled: ${tasks} but no agent activity for ${minutes} min`;
}

export interface BacklogChangeEvent {
  kind: tasksChangeKind;
  itemId: number | null;
}

export function subscribeBacklogEvents(onEvent: (event: BacklogChangeEvent) => void): () => void {
  // Backlog == tasks not attached to a thread. The backend
  // collapses both onto `tasksChanged { threadId? }`; threadId is
  // null for backlog rows. The bus event no longer carries kind/itemId
  // so we synthesize a coarse "updated" — receivers refetch.
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "tasksChanged") return;
    if (event.threadId != null) return;
    onEvent({ kind: "updated", itemId: null });
  });
}

export function subscribeTaskEvents(
  _streamId: string | "all",
  onEvent: (event: tasksChangeEvent) => void,
): () => void {
  // The backend `tasksChanged` payload only carries `threadId`
  // (no streamId / itemId / kind), so we can't honour the streamId
  // filter or report which item changed. Fire a coarse "updated"
  // for every thread-scoped tasks change — receivers refetch.
  // The streamId filter parameter is preserved for API compatibility
  // but is currently a no-op.
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "tasksChanged") return;
    const threadId = event.threadId as string | undefined | null;
    if (!threadId) return;
    onEvent({
      streamId: "",
      threadId,
      kind: "updated",
      itemId: null,
    });
  });
}

export async function probeDaemon(): Promise<boolean> {
  try {
    unwrap(await commands.ping());
    return true;
  } catch {
    return false;
  }
}

export type NormalizedEvent =
  | { kind: "session-start"; t: number; sessionId?: string; cwd?: string }
  | { kind: "session-end"; t: number; sessionId?: string; reason?: string }
  | { kind: "user-prompt"; t: number; sessionId?: string; prompt: string }
  | {
      kind: "tool-use-start";
      t: number;
      sessionId?: string;
      toolName: string;
      target?: string;
      input?: unknown;
    }
  | {
      kind: "tool-use-end";
      t: number;
      sessionId?: string;
      toolName: string;
      status: "ok" | "error";
    }
  | { kind: "stop"; t: number; sessionId?: string }
  | { kind: "notification"; t: number; sessionId?: string; message: string }
  | { kind: "meta"; t: number; sessionId?: string; hookEventName: string; raw: unknown };

export interface StoredEvent {
  id: number;
  streamId: string;
  threadId?: string;
  pane?: "working" | "talking";
  normalized: NormalizedEvent;
}

export async function listHookEvents(_streamId?: string): Promise<StoredEvent[]> {
  return unwrap(
    await commands.listHookEvents(null, null),
  ) as unknown as StoredEvent[];
}

export function subscribeHookEvents(
  streamId: string | "all",
  onEvent: (event: StoredEvent) => void,
): () => void {
  // Backend `HookEventsChanged` is a coarse "something landed" ping —
  // no payload. Refetch the latest hook event and forward it; this
  // misses bursts but matches the renderer's "refetch on signal" model.
  let lastSeenId = -1;
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "hookEventsChanged") return;
    void listHookEvents().then((events) => {
      if (events.length === 0) return;
      // Events are returned newest-first by listHookEvents.
      const next = events[0];
      if (typeof next.id === "number" && next.id <= lastSeenId) return;
      if (typeof next.id === "number") lastSeenId = next.id;
      if (streamId !== "all" && next.streamId !== streamId) return;
      onEvent(next);
    });
  });
}

/**
 * Bridge facade exposing the runtime IPC methods that need
 * lifecycle wrapping (menu / lsp / terminal / external-url /
 * logUi). Lazily built on first access; every caller shares
 * the same instance. Read-only RPC stays on the top-level
 * wrapper functions in this file.
 */
export function desktopBridge(): DesktopBridge {
  if (!cachedBridge) cachedBridge = buildBridge();
  return cachedBridge;
}

/**
 * Open an http(s) URL in the user's OS browser. The main process
 * re-validates the URL against the same scheme allowlist as the
 * renderer; non-allowed URLs return `{ ok: false }` so callers can
 * show a refusal toast.
 */
export async function installLspPackage(packageName: string): Promise<InstalledLspPackage> {
  return unwrap(await commands.installLspPackage(packageName));
}

export async function listInstalledLspPackages(): Promise<InstalledLspPackage[]> {
  return unwrap(await commands.listInstalledLspPackages());
}

/// JSON-RPC request on the shared backend LSP session for
/// (stream, language). Payloads cross the boundary as JSON strings
/// (specta can't type serde_json::Value cleanly); the (de)serialization
/// is contained here.
export async function lspRequest(
  streamId: string,
  languageId: string,
  method: string,
  params: unknown,
): Promise<unknown> {
  const result = unwrap(
    await commands.lspRequest(streamId, languageId, method, JSON.stringify(params ?? {})),
  );
  return JSON.parse(result);
}

export async function lspNotify(
  streamId: string,
  languageId: string,
  method: string,
  params: unknown,
): Promise<void> {
  unwrap(await commands.lspNotify(streamId, languageId, method, JSON.stringify(params ?? {})));
}

export async function listLspServers(): Promise<LspServerListing[]> {
  return unwrap(await commands.listLspServers());
}

export async function restartLspServer(streamId: string, languageId: string): Promise<void> {
  unwrap(await commands.restartLspServer(streamId, languageId));
}

export async function removeLspPackage(packageName: string): Promise<void> {
  unwrap(await commands.removeLspPackage(packageName));
}

/// Answer a server-initiated workspace/applyEdit forwarded over the
/// lsp:event channel (kind "applyEditRequest"). Late answers are no-ops
/// backend-side.
export async function respondLspApplyEdit(
  token: number,
  applied: boolean,
  failureReason?: string,
): Promise<void> {
  unwrap(await commands.respondLspApplyEdit(token, applied, failureReason ?? null));
}

export async function openExternalUrl(url: string): Promise<{ ok: boolean; reason?: string }> {
  try {
    unwrap(await commands.openExternalUrl(url));
    return { ok: true };
  } catch (e) {
    return { ok: false, reason: e instanceof Error ? e.message : String(e) };
  }
}

// ----------------------------------------------------------------------
// Unified page-ref graph (cross-page backlinks + outbound).
// ----------------------------------------------------------------------

export type BacklinkEdge = import("./tauri-bridge/generated/bindings.js").BacklinkEdge;

/** Pages pointing AT (target_kind, target_id). */
export async function listBacklinks(
  targetKind: string,
  targetId: string,
  limit: number | null = null,
): Promise<BacklinkEdge[]> {
  return unwrap(await commands.listBacklinks(targetKind, targetId, limit));
}

/** Pages this source points AT. Inverse of `listBacklinks`. */
export async function listPageOutbound(
  sourceKind: string,
  sourceId: string,
  limit: number | null = null,
): Promise<BacklinkEdge[]> {
  return unwrap(await commands.listOutbound(sourceKind, sourceId, limit));
}
