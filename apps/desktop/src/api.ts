import { commands } from "./tauri-bridge/generated/bindings.js";
import type {
  BackgroundTask,
  TabReport,
  HookEnvelope,
  OpOutcome,
  OxplowConfig,
  OxplowEvent,
  Reads,
  SymbolLocation,
  SnapshotTrigger,
  FsStat,
} from "./tauri-bridge/generated/bindings.js";
import { listen, onRemoteReconnect, sendSocketFrame, triggerRemoteResync } from "./tauri-bridge/transport.js";

/** Whether this window is in the desktop shell — false in a plain browser
 *  driving a daemon, which has no native menu or recent projects. */
export { shellAvailable } from "./tauri-bridge/transport.js";

export { onRemoteReconnect, triggerRemoteResync };
import { EVENT_CHANNELS, type EventChannelFrameKey } from "./tauri-bridge/channels.js";
import {
  METRIC_SPECS_SQL,
  metricSpecs as metricSpecRows,
  type MetricSpec,
} from "./metricsSql.js";
import { IpcCallError, ipcErrorCode, ipcErrorDetails, ipcErrorField, ipcErrorMessage } from "./ipc-error.js";
import type {
  AiSettings,
  AssistantContext,
  AssistantModel,
  AssistantSnapshot,
  HarnessListing,
  PanelPlacement,
  UiPreference,
  CommentIntent,
  CommentMessage,
  CommentStatus,
  CommentThread,
  Extension,
  VizConfig,
  VizRun,
  Viz,
  RecentProjectView,
  RoleBinding,
  ServiceListing,
  CommandOutcome,
  CommandSpec,
  FormStart,
  SearchHit,
  SqlCell,
  SqlQueryResult,
} from "./tauri-bridge/generated/bindings.js";

export type { AiSettings, RoleBinding };

/** A provider as `oxplow.ai.set_provider` takes it. */
export interface ProviderConfig {
  id: string;
  kind: string;
  baseUrl: string | null;
}
export type { ServiceListing };
export type { Extension, VizConfig, VizRun, Viz, SearchHit, SqlCell, SqlQueryResult };

/// Convert the tauri-specta {status, data|error} envelope into a
/// plain promise return. Errors are usually IpcError objects with
/// message/code, but arg-deserialization failures and panics arrive as
/// a plain string — `ipcErrorMessage` surfaces the real reason verbatim
/// instead of collapsing to a generic "ipc error" (see ipc-error.ts).
/** A generated command's result, or its error thrown. */
export function unwrapResult<T>(result: { status: "ok"; data: T } | { status: "error"; error: unknown }): T {
  return unwrap(result);
}

function unwrap<T>(result: { status: "ok"; data: T } | { status: "error"; error: unknown }): T {
  if (result.status === "ok") return result.data;
  throw new IpcCallError(
    ipcErrorMessage(result.error),
    ipcErrorCode(result.error),
    ipcErrorField(result.error),
    ipcErrorDetails(result.error),
  );
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
      groups: import("./tauri-bridge/generated/bindings.js").MenuGroupSnapshot[],
    ): Promise<void> => {
      try {
        unwrap(await commands.setNativeMenu(groups));
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
    /// The process behind `uri` (a terminal's shell, an agent session's CLI),
    /// started if it isn't running; opening it again resumes the one.
    openTerminalSession: async (
      uri: string,
      cols: number,
      rows: number,
    ): Promise<{ sessionId: string; replayB64: string }> => {
      const result = unwrap(await commands.openTerminalSession(uri, cols, rows));
      return { sessionId: result.sessionId, replayB64: result.replayB64 };
    },
    /// The live process of `uri` (a terminal or an agent session), without
    /// starting one: `null` when none runs.
    lookupTerminalSession: async (uri: string): Promise<string | null> =>
      unwrap(await commands.lookupTerminalSession(uri)),
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
    // Plumbing for HUMAN terminal input only — the xterm in TerminalSurface
    // pipes the user's keystrokes / paste / scroll / resize through here.
    // NOT an agent-messaging or automation API; never synthesize
    // `{type:"input"}` from non-UI code (see .context/agent-model.md).
    // One frame on the daemon's socket, so keys reach the PTY of `uri` in
    // the order they were typed; the daemon holds them until it is up.
    terminalInput: (uri: string, message: string): void => {
      sendSocketFrame(JSON.stringify({ channel: "terminal" satisfies EventChannelFrameKey, uri, message }));
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
    /// `clipboardReadText` is read by `TerminalSurface`'s Cmd+V paste: the
    /// shell's clipboard, since a webview's own read may be refused.
    clipboardReadText: async (): Promise<string> =>
      unwrap(await commands.clipboardReadText()),
  };
}

export type DesktopBridge = ReturnType<typeof buildBridge>;
let cachedBridge: DesktopBridge | null = null;

// The wire's own shapes, from the generated bindings: nothing converts a
// shape at the boundary (P11, tsk963–964).
export type { BackgroundTask, OxplowConfig, OxplowEvent };
export type { OpOutcome } from "./tauri-bridge/index.js";


// Stream / Thread come straight from the Tauri bindings — the
// renderer reads the flat shape (working_pane / talking_pane /
// custom_prompt) directly; no synthesis happens at the boundary.
import type { Stream, Thread, ThreadMark } from "./tauri-bridge/index.js";
export type { Stream, Thread, ThreadMark };

export interface ThreadState {
  selectedThreadId: string | null;
  threads: Thread[];
}

// Work items are read through the work-item interface by `workItems.ts`,
// which owns their shape.
export type { WorkItem } from "./workItems.js";


/** Why a snapshot take ran (`snapshot_op.trigger`). */
export type SnapshotSource = SnapshotTrigger;

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


export interface TaskEffort {
  id: string;
  /** The work item worked on (`work_item:oxplow:tsk42`, or another
   *  provider's `work_item:issues:ENG-12`). */
  work_item: string;
  started_at: string;
  ended_at: string | null;
  start_snapshot_id: string | null;
  end_snapshot_id: string | null;
  /** The effort's summary prose (canonical text). */
  summary: string | null;
}

export const BACKLOG_SCOPE = "__backlog__";



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

import type { Revision } from "./revision.js";
import {
  STREAM_HEAD_SQL,
  streamHeadFromRows,
  type StreamHead,
} from "./streamHead.js";
import { streamRowId } from "./modelIds.js";
import type { BlameLine } from "./tauri-bridge/generated/bindings.js";
export type { BlameLine };

// ---- The stream's version control, neutral (`.context/vcs.md`) ----

/** Where the stream's workspace is: its head revision and branch, read
 *  from `v_stream` / `v_branch` (restated whenever refs move). */
export async function streamHead(streamId: string): Promise<StreamHead> {
  const res = await querySql(STREAM_HEAD_SQL, [streamRowId(streamId)], 1);
  return streamHeadFromRows(res.rows);
}

/** Who last changed each line of `path` at `revision` (the working tree:
 *  uncommitted lines have no revision). */
export async function vcsBlame(
  streamId: string,
  path: string,
  revision: Revision,
): Promise<BlameLine[]> {
  return unwrap(await commands.vcsBlame(streamId || null, path, revision));
}

import type { InstalledLspPackage, LspServerListing } from "./tauri-bridge/generated/bindings.js";
export type { InstalledLspPackage, LspServerListing };

export interface WorkspaceContext {
  vcsEnabled: boolean;
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
/// everything. `resources` optionally restricts to searchable resources
/// (`v_resource.searchable`: `oxplow/wiki`) or `oxplow/file`.
export async function searchSite(
  query: string,
  streamId: string | null,
  resources: string[] | null = null,
  limit = 50,
): Promise<SearchHit[]> {
  return unwrap(await commands.search(query, streamId, resources, limit));
}

/// Run one read-only `SELECT`/`WITH` over the semantic layer's `v_*`
/// views (see `.context/semantic-layer.md`). Positional `params` bind
/// `?1`, `?2`, …; `limit` caps rows (default 500).
/** One read over the published datasets. `raw` (the explorer only) reads
 *  physical tables too; the result's `reads` says what it read. */
export async function querySql(
  sql: string,
  params: SqlCell[] = [],
  limit: number | null = null,
  raw = false,
): Promise<SqlQueryResult> {
  return unwrap(await commands.querySql(sql, params, limit, raw));
}

/// The extensions the app shows (and their pages) — the main worktree's,
/// for every stream — with per-extension load errors. See
/// `.context/extensions.md`.
export async function listExtensions(): Promise<Extension[]> {
  return unwrap(await commands.listExtensions());
}

/** A command's spec: what a form renders from its `input_schema`, and
 *  what a confirmation says. */
export async function getCommand(name: string): Promise<CommandSpec> {
  return unwrap(await commands.getCommand(name));
}

/** Load component `id`'s bundle (`<extension>/<component id>`) as it is
 *  now, for its frame (tsk984): the version the daemon serves it at and
 *  the frame invokes with. */
export async function loadComponent(id: string): Promise<string> {
  return unwrap(await commands.loadComponent(id));
}

/** A custom component's frame invokes one of its declared commands, as the
 *  component acting for the person; `confirmed` only after the host asked;
 *  `version` is the bundle the frame was loaded at, which must be approved. */
export async function invokeComponentCommand(
  id: string,
  command: string,
  input: unknown,
  confirmed: boolean,
  version: string,
): Promise<CommandOutcome> {
  return unwrap(await commands.invokeComponentCommand(id, command, input as never, confirmed, version));
}

/** A custom component that declares `sql: read` runs its own read-only SQL
 *  over the datasets (`querySql`), at the bundle version its frame loaded. */
export async function runComponentSql(
  id: string,
  sql: string,
  params: SqlCell[],
  streamId: string | null,
  version: string,
): Promise<SqlQueryResult> {
  return unwrap(await commands.runComponentSql(id, sql, params, streamId, version));
}

/** The form of one of component `id`'s commands, for the host to show when
 *  its frame asks (`openForm`). */
export async function componentForm(id: string, command: string, input: unknown, version: string): Promise<FormStart> {
  return unwrap(await commands.componentForm(id, command, (input ?? null) as never, version));
}

/** What the person can ask: every capability's questions and the enabled
 *  extensions' prompts (the catalog; contextual suggestions). */

/** The person's left-nav layout: each panel's order, and whether it's
 *  hidden or collapsed (P6.G1). */
export async function getPanelLayout(): Promise<PanelPlacement[]> {
  return unwrap(await commands.getPanelLayout());
}

export async function setPanelLayout(layout: PanelPlacement[]): Promise<void> {
  unwrap(await commands.setPanelLayout(layout));
}

/** What the person's oxplow remembers outside any one tab: page memory
 *  and app preferences (`ui_preference`; `prefs/remembered.ts`). */
export async function getUiPreferences(): Promise<UiPreference[]> {
  return unwrap(await commands.getUiPreferences());
}

/** Remember one value (JSON text), or forget it with null. */
export async function setUiPreference(scope: string, key: string, valueJson: string | null): Promise<void> {
  unwrap(await commands.setUiPreference(scope, key, valueJson));
}

/** Every service a script may be given, by title (`list_services`). */
export async function listServices(): Promise<ServiceListing[]> {
  return unwrap(await commands.listServices());
}


/// Tell the backend what this window shows (`tabs/tabReport.ts`), for an
/// agent's `list_tabs` / `get_open_page`.
export async function reportTabs(report: TabReport): Promise<void> {
  unwrap(await commands.reportTabs(report));
}

/// Analyze a change (a commit, an effort, or a stream's working tree) if
/// needed and return its `v_change` row. See `.context/semantic-layer.md`.

export { openInSystemBrowser } from "./tauri-bridge/systemBrowser.js";
export {
  answerSignInRedirect,
  awaitSignInRedirect,
  canCatchSignInRedirect,
  listenForSignInRedirect,
  stopSignInRedirect,
  type SignInListener,
} from "./tauri-bridge/oauthRedirect.js";

/// The registered agent harnesses in priority order: the session picker's
/// and Settings' (`agentKinds.ts`).
export async function listAgentHarnesses(): Promise<HarnessListing[]> {
  return unwrap(await commands.listAgentHarnesses());
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
  return (await runCommand("oxplow.stream.rename", { stream: streamId, title })).result as Stream;
}

/** Archive a stream. The Remove dialog is the person's confirmation
 *  (`oxplow.stream.archive` is destructive), so this runs it confirmed. */
export async function archiveStream(streamId: string, deleteWorktree: boolean): Promise<void> {
  await runCommand("oxplow.stream.archive", { stream: streamId, delete_worktree: deleteWorktree }, true);
}

export async function renameCurrentStream(title: string): Promise<Stream> {
  const cur = unwrap(await commands.getCurrentStream());
  if (!cur) throw new Error("no current stream to rename");
  return renameStream(cur.id, title);
}

export async function getConfig(): Promise<OxplowConfig> {
  return unwrap(await commands.getConfig());
}

/// The enabled agent harnesses, by key, in priority order.


/// Set (or clear, with null/blank) one of a harness's declared settings —
/// `agentConfig.<agent>.<key>` in .oxplow/project.yaml, which its launch
/// reads (the harness's listing names its settings).

export type CommitRefLabel = import("./tauri-bridge/generated/bindings.js").CommitRefLabel;

export async function resolveCommitRefLabels(
  shas: string[],
): Promise<Record<string, CommitRefLabel[]>> {
  if (shas.length === 0) return {};
  return unwrap(await commands.gitResolveCommitRefLabels(shas));
}

/**
 * Long-running git ops are kickoff-style — the IPC promise resolves
 * immediately with a `taskId` once the BackgroundTaskStore row is
 * registered, and the actual work runs in the background. Each
 * renderer-side wrapper also exposes an `awaitDone` promise that
 * resolves with the final `BackgroundTask` (status, error, and the
 * `result_json` payload — the command's `OpOutcome`, read with
 * `taskResult`). Pattern:
 *
 *     const { taskId, awaitDone } = await gitRebase(...);
 *     // mark UI pending using taskId / a label
 *     const task = await awaitDone;
 *     // taskResult(task) is the OpOutcome
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
/// row so `awaitDone` resolves with the actual OpOutcome and the
/// shared "in-flight task" subscribers stay accurate. Without
/// this, the renderer's kickoff pattern (vcsPush / vcsPull etc.)
/// would race a never-completing fake task and the result would
/// land in the void.
async function runAsBackgroundTask(
  label: string,
  kind: import("./tauri-bridge/index.js").BackgroundTaskKind,
  detail: string | null,
  op: () => Promise<OpOutcome>,
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

/** The commands a person is offered — those with a `ui` they may run now
 *  (`list_person_commands`); search lists the ref-less ones. */
/** Window `client` says it's open and hosts `scopes` (`clientHost.ts`). */
export async function registerClientHost(client: string, scopes: string[]): Promise<void> {
  unwrap(await commands.registerClientHost(client, scopes));
}

/** Window `client` is closing: calls stop coming to it. */
export async function unregisterClientHost(client: string): Promise<void> {
  unwrap(await commands.unregisterClientHost(client));
}

/** Window `client`'s answer to a `clientCall`: its result, or why not. */
export async function answerClientCall(
  client: string,
  id: string,
  answer: { result: unknown } | { error: string },
): Promise<void> {
  unwrap(
    await commands.answerClientCall(
      client,
      id,
      "result" in answer ? (answer.result ?? null) : null,
      "error" in answer ? answer.error : null,
    ),
  );
}

/** Run command `name` while answering the daemon's call `call`, as whoever
 *  made it (an agent's Save writes the file as the agent). */
export async function runCommandForCall(
  call: { client: string; id: string },
  name: string,
  input: unknown,
): Promise<CommandOutcome> {
  return unwrap(await commands.runCommandForCall(call.client, call.id, name, input));
}

export async function listPersonCommands(): Promise<CommandSpec[]> {
  return unwrap(await commands.listPersonCommands());
}

/** The ids of the commands about `uri` offered on it now: those about its
 *  kind (`ui.about`) whose `ui.about_when` holds on its row
 *  (`ref_commands`). */
export async function refCommands(uri: string): Promise<string[]> {
  return unwrap(await commands.refCommands(uri));
}

/** Run command `id` as a background task (its `ui.background`: a slow
 *  call out). Resolves once it started; `awaitGitOp` on the kickoff for
 *  its outcome. */
export async function runCommandInBackground(label: string, id: string, input: unknown): Promise<GitOpKickoff> {
  const kind = id.startsWith("oxplow.vcs.") ? "vcs" : "command";
  return runAsBackgroundTask(label, kind, id, async () => (await runCommand(id, input)).result as OpOutcome);
}

export async function vcsMerge(streamId: string, rev: string, confirmed: boolean): Promise<GitOpKickoff> {
  return runAsBackgroundTask(`Merge ${rev}`, "vcs", `merge ${rev}`, () =>
    runVcs("oxplow.vcs.merge", { stream: streamId, rev }, confirmed),
  );
}

export async function gitRebase(streamId: string, onto: string, confirmed: boolean): Promise<GitOpKickoff> {
  return runAsBackgroundTask(`Rebase onto ${onto}`, "vcs", `rebase ${onto}`, () =>
    runVcs("oxplow.git.rebase", { stream: streamId, rev: onto }, confirmed),
  );
}

export async function gitCherryPick(streamId: string, rev: string): Promise<GitOpKickoff> {
  const short = rev.slice(0, 7);
  return runAsBackgroundTask(`Cherry-pick ${short}`, "vcs", `cherry-pick ${short}`, () =>
    runVcs("oxplow.git.cherry_pick", { stream: streamId, rev }),
  );
}

export async function gitRevert(streamId: string, rev: string, confirmed: boolean): Promise<GitOpKickoff> {
  const short = rev.slice(0, 7);
  return runAsBackgroundTask(`Revert ${short}`, "vcs", `revert ${short}`, () =>
    runVcs("oxplow.git.revert", { stream: streamId, rev }, confirmed),
  );
}

export async function getWorkspaceContext(): Promise<WorkspaceContext> {
  const ctx = unwrap(await commands.getWorkspaceContext());
  return { vcsEnabled: ctx.vcs_enabled };
}

// ---- Launcher / multi-window ----

/// Recent projects for the launcher, most-recent first, each tagged
/// with whether its directory still exists on disk.
export async function listRecentProjects(): Promise<RecentProjectView[]> {
  return unwrap(await commands.listRecentProjects());
}

/// Put the `oxplow` command on the PATH (a link to this app); the hint, when
/// set, says what to add to the PATH.
export async function installCli(): Promise<{ path: string; hint: string | null }> {
  return unwrap(await commands.installCli());
}

/// Forget a project from the recent list.
export async function removeRecentProject(path: string): Promise<void> {
  unwrap(await commands.removeRecentProject(path));
}

/// Open an existing project at `path`. `newWindow=false` replaces the
/// current window (this process exits once the new one is spawned);
/// `newWindow=true` opens an additional independent window. A folder
/// that isn't a project yet errors — use `createProject` for that.
export async function openProject(path: string, newWindow: boolean): Promise<void> {
  unwrap(await commands.openProject(path, newWindow));
}

/// Create a new project in `path` (initializes `.oxplow/`) and open it
/// in a new window. Errors if `path` is already a project.
export async function createProject(path: string): Promise<void> {
  unwrap(await commands.createProject(path));
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

/** Switch the stream's workspace to `branch` (`create` makes it at the
 *  head first). The branch reconciler records it on the stream. */
export async function vcsCheckoutBranch(streamId: string, branch: string, create = false): Promise<OpOutcome> {
  return runVcs("oxplow.vcs.checkout_branch", { stream: streamId, name: branch, create });
}

export async function vcsRenameBranch(streamId: string, from: string, to: string): Promise<OpOutcome> {
  return runVcs("oxplow.vcs.rename_branch", { stream: streamId, from, to });
}

export async function vcsDeleteBranch(
  streamId: string,
  name: string,
  force: boolean,
  confirmed: boolean,
): Promise<OpOutcome> {
  return runVcs("oxplow.vcs.delete_branch", { stream: streamId, name, force }, confirmed);
}

export async function getThreadState(streamId: string): Promise<ThreadState> {
  return unwrap(await commands.getThreadState(streamId)) as unknown as ThreadState;
}


/** A new thread on the stream: it has no agent session (open one with
 *  `openAgentSession`). The thread and the stream's state after. */
export async function createThread(streamId: string, title: string): Promise<{ thread: Thread; state: ThreadState }> {
  const thread = (await runCommand("oxplow.thread.create", { stream: streamId, title })).result as Thread;
  return { thread, state: await getThreadState(streamId) };
}

/** What `oxplow.agent_session.open` answers: the new session's row. */
export interface AgentSessionRecord {
  /** `ses3`. */
  id: string;
  /** `thr1`. */
  thread_id: string;
  kind: string;
  /** The harness's key. */
  harness: string;
  title: string;
}

/** Open an agent session on a thread: it only adds the slot (the tab
 *  starts its process when it mounts). No harness: the project's default. */
export async function openAgentSession(
  threadId: string,
  harness?: string,
  title?: string,
  /** A saved conversation of the harness (its native id) to start on. */
  resume?: string,
): Promise<AgentSessionRecord> {
  return (
    await runCommand("oxplow.agent_session.open", {
      thread: threadId,
      ...(harness ? { harness } : {}),
      ...(title ? { title } : {}),
      ...(resume ? { resume } : {}),
    })
  ).result as AgentSessionRecord;
}

/** The person gives an agent a new conversation, dropping the saved one.
 *  Destructive: the caller confirms first. */
export async function startAgentSessionFresh(sessionId: string, confirmed: boolean): Promise<void> {
  await runCommand("oxplow.agent_session.start_fresh", { session: sessionId }, confirmed);
}

export async function selectThread(streamId: string, threadId: string): Promise<ThreadState> {
  unwrap(await commands.selectThread({ streamId, threadId }));
  return getThreadState(streamId);
}

/** Mark a thread's tab with a colour, or clear it (`null`). */
export async function markThread(
  streamId: string,
  threadId: string,
  mark: ThreadMark | null,
): Promise<ThreadState> {
  await runCommand("oxplow.thread.mark", { thread: threadId, ...(mark ? { mark } : {}) });
  return getThreadState(streamId);
}

export async function closeThread(streamId: string, threadId: string): Promise<ThreadState> {
  await runCommand("oxplow.thread.close", { thread: threadId });
  return getThreadState(streamId);
}

export async function renameThread(_streamId: string, threadId: string, title: string): Promise<Thread> {
  return (await runCommand("oxplow.thread.rename", { thread: threadId, title })).result as Thread;
}

export async function setStreamPrompt(streamId: string, prompt: string | null): Promise<Stream[]> {
  await runCommand("oxplow.stream.set_prompt", { stream: streamId, ...(prompt ? { prompt } : {}) });
  return listStreams();
}

export async function setThreadPrompt(
  _streamId: string,
  threadId: string,
  prompt: string | null,
): Promise<Thread[]> {
  await runCommand("oxplow.thread.set_prompt", { thread: threadId, ...(prompt ? { prompt } : {}) });
  return [];
}

/** A remote branch to push to or pull from; omitted = the upstream. */
export interface RemoteBranchTarget {
  remote: string;
  branch: string;
}

/** Write a wiki page — the `oxplow.knowledge.write_page` command (P5.C3): the
 *  row, its links and the file, in one audited run. Every `[[link]]` must
 *  resolve (a dangling one is refused, named). `verifiedFiles` are files
 *  re-checked against the page: their freshness pins move to now. */
export async function writeWikiPage(
  slug: string,
  body: string,
  options?: { verifiedFiles?: string[] },
): Promise<void> {
  await runCommand("oxplow.knowledge.write_page", {
    slug,
    body,
    verified_files: options?.verifiedFiles ?? [],
  });
}

/** Delete a wiki page (`oxplow.knowledge.delete_page`, destructive): the person
 *  has confirmed. */
export async function deleteWikiPage(slug: string, confirmed: boolean): Promise<void> {
  await runCommand("oxplow.knowledge.delete_page", { slug }, confirmed);
}

// ---- Comments ----

/** A comment, written as the person (`oxplow.knowledge.add_comment`, P8.A6). */
export async function createComment(input: {
  streamId: string;
  threadId: string | null;
  /** The URI the comment is on. */
  target: string;
  quote: string;
  /** W3C selectors array, serialized. */
  selectorsJson: string;
  /** Ancestor regions' URIs (innermost→outermost, excluding the target). */
  contextChain?: string[];
  /** URIs found inside the selection. */
  referenced?: string[];
  intent: CommentIntent;
  body: string;
}): Promise<CommentThread> {
  const outcome = await runCommand("oxplow.knowledge.add_comment", {
    stream: input.streamId,
    ...(input.threadId ? { thread: input.threadId } : {}),
    target: input.target,
    quote: input.quote,
    selectors_json: input.selectorsJson,
    context_chain: input.contextChain ?? [],
    referenced: input.referenced ?? [],
    intent: input.intent,
    body: input.body,
  });
  return outcome.result as CommentThread;
}

export async function addCommentMessage(commentId: string, body: string): Promise<CommentMessage> {
  return (await runCommand("oxplow.knowledge.reply_comment", { comment: commentId, body })).result as CommentMessage;
}

export async function listCommentsForTarget(target: string): Promise<CommentThread[]> {
  return unwrap(await commands.listCommentsForTarget(target));
}

export async function listCommentsForStream(streamId: string): Promise<CommentThread[]> {
  return unwrap(await commands.listCommentsForStream(streamId));
}

export async function setCommentIntent(commentId: string, intent: CommentIntent): Promise<void> {
  await runCommand("oxplow.knowledge.update_comment", { comment: commentId, intent });
}

export async function setCommentStatus(commentId: string, status: CommentStatus): Promise<void> {
  await runCommand("oxplow.knowledge.update_comment", { comment: commentId, status });
}

/// Store where the renderer re-found a comment's quote (or that it is
/// gone). An anchor already where it was leaves no record.
export async function relocateComment(
  commentId: string,
  selectorsJson: string,
  orphaned: boolean,
): Promise<void> {
  await runCommand("oxplow.knowledge.relocate_comment", { comment: commentId, selectors_json: selectorsJson, orphaned });
}

/// Re-attach an orphaned comment to a freshly-selected span: rewrites
/// both quote + anchor and clears the orphan flag.
export async function relinkComment(
  commentId: string,
  quote: string,
  selectorsJson: string,
): Promise<void> {
  await runCommand("oxplow.knowledge.update_comment", { comment: commentId, quote, selectors_json: selectorsJson });
}

/** The popover's delete sits behind an `InlineConfirm`, the person's
 *  confirmation (`oxplow.knowledge.delete_comment` is destructive). */
export async function deleteComment(commentId: string): Promise<void> {
  await runCommand("oxplow.knowledge.delete_comment", { comment: commentId }, true);
}

/// Subscribe to comment changes: a commit touched `v_comment` (a comment
/// or one of its messages), whoever wrote it. A view re-reads its list.
export function subscribeCommentEvents(onChange: () => void): () => void {
  return subscribeOxplowEvents((event) => {
    const datasets = (event as { datasets?: unknown }).datasets;
    if (event.kind === "datasetsChanged" && Array.isArray(datasets) && datasets.includes("v_comment")) {
      onChange();
    }
  });
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

/** Call `onChange` whenever a commit names one of `datasets` — how a view
 *  re-reads what it shows, whoever wrote it. */
function onDatasetsChanged(datasets: readonly string[], onChange: () => void): () => void {
  return subscribeOxplowEvents((event) => {
    const changed = (event as { datasets?: unknown }).datasets;
    if (
      event.kind === "datasetsChanged" &&
      Array.isArray(changed) &&
      changed.some((m) => datasets.includes(m as string))
    ) {
      onChange();
    }
  });
}

export async function listBackgroundTasks(): Promise<BackgroundTask[]> {
  return unwrap(await commands.listBackgroundTasks());
}

export async function getBackgroundTask(id: string): Promise<BackgroundTask | null> {
  return unwrap(await commands.getBackgroundTask(id));
}

/** What a finished task's producer attached (`result_json`, parsed);
 *  `undefined` when it attached nothing readable. */
export function taskResult(task: BackgroundTask): unknown {
  if (!task.result_json) return undefined;
  try {
    return JSON.parse(task.result_json);
  } catch {
    return undefined;
  }
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
 * task row so callers can inspect `task.status`, `task.error`, and its
 * result (`taskResult(task)`). Returns null if the task disappeared (evicted) before
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

export type { MetricSpec, SeriesPoint } from "./metricsSql.js";

import { effortUri, idOf } from "./uri/uri.js";

/** Rows a metric reader returned, and what the query read — hand `reads` to
 *  `useRerunOnChange` so the view refreshes when (and only when) they change. */
export interface MetricRows<T> {
  rows: T[];
  reads: Reads;
}

/** Every metric definition — `v_metric_spec` (P4.7: metrics read through SQL). */
export async function listMetricDefinitions(): Promise<MetricRows<MetricSpec>> {
  const result = await querySql(METRIC_SPECS_SQL, [], 10_000);
  return { rows: metricSpecRows(result), reads: result.reads };
}

/** Run a command on the bus as the person (P5.A1). `confirmed` says the
 *  person confirmed this exact call; a call that needs one comes back
 *  `NEEDS_CONFIRMATION`, so ask and call again. */
export async function runCommand(name: string, input: unknown, confirmed = false): Promise<CommandOutcome> {
  return unwrap(await commands.runCommand(name, input, confirmed));
}

/** The page command `name` opens after a run that gave `result`: its
 *  `ui.open_after`, bound by the server from the result; `null` when it
 *  says none. A field the result lacks is an error naming it. */
export async function commandOpenAfter(name: string, result: unknown): Promise<string | null> {
  return unwrap(await commands.commandOpenAfter(name, result));
}

/** Run a `vcs.*` / `git.*` command (P5.B6) and return its `OpOutcome`.
 *  A destructive one is refused `NEEDS_CONFIRMATION` unless the caller
 *  asked the person first and passes `confirmed`. */
async function runVcs(name: string, input: unknown, confirmed = false): Promise<OpOutcome> {
  return (await runCommand(name, input, confirmed)).result as OpOutcome;
}

/** Undo the run recorded as `auditId`, as the person. */
export async function undoCommand(auditId: number, confirmed = false): Promise<CommandOutcome> {
  return unwrap(await commands.undoCommand(auditId, confirmed));
}

/** Approve (it runs as the person; its outcome) or decline (`null`) an
 *  agent's proposal `proposal:<id>`. */
export async function decideProposal(id: number, approve: boolean): Promise<CommandOutcome | null> {
  return unwrap(await commands.decideProposal(id, approve));
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


/** Snapshot row — one per `request_snapshot()` call that captured
 *  anything. Local History dashboard surfaces this list. */
export interface Snapshot {
  id: number;
  streamId: string;
  createdAt: string;
  fileCount: number;
  /** The VCS revision the snapshot's tree equals (`git:<sha>`) — set when
   *  it was taken on a clean workspace at its head. */
  revision: Revision | null;
  /** The branch the workspace was on at capture; null for a
   *  detached head or a directory not under version control. */
  branch: string | null;
  /** What the take that created it recorded (P2.11): the snapshot it grew
   *  from, why it was taken, and whether it ran over its time budget. */
  parentSnapshotId: number | null;
  trigger: SnapshotTrigger;
  overBudget: boolean;
}

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

/** Every file in the stream's workspace at `revision`, sorted. */
export async function filesAt(streamId: string, revision: Revision): Promise<string[]> {
  return unwrap(await commands.filesAt(streamId || null, revision));
}

/** One (snapshot, effort) pair. `completedHere` is true when the
 *  effort ended exactly at this snapshot; otherwise the effort was
 *  in flight at this snapshot. Callers group by `snapshotId` and
 *  resolve task titles via `getTaskSummaries` (the IPC only carries
 *  effort columns). */
export interface EffortAtSnapshot {
  snapshotId: number;
  effortId: string;
  /** The effort's work item ref; `null` while it's unlinked. */
  workItem: string | null;
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

/** Efforts' titles (`v_effort.title`: their own, else their item's, else
 *  their first prompt's first line), by effort id (`eff12`). */
export async function effortTitles(effortIds: string[]): Promise<Map<string, string>> {
  if (effortIds.length === 0) return new Map();
  const res = await querySql(
    `SELECT uri, title FROM v_effort WHERE uri IN (SELECT value FROM json_each(?1)) AND title IS NOT NULL`,
    [JSON.stringify(effortIds.map(effortUri))],
    10_000,
  );
  return new Map(res.rows.map((r) => [idOf(String(r[0]), "oxplow/effort") ?? "", String(r[1])] as [string, string]));
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
      work_item: string | null;
      thread_id: string;
      start_snapshot_id: number | null;
      end_snapshot_id: number | null;
    };
  }>;
  return rows.map((r) => ({
    snapshotId: r.snapshot_id,
    effortId: r.effort.id,
    workItem: r.effort.work_item,
    threadId: r.effort.thread_id,
    startSnapshotId: r.effort.start_snapshot_id,
    endSnapshotId: r.effort.end_snapshot_id,
    completedHere: r.effort.end_snapshot_id === r.snapshot_id,
  }));
}

export interface SnapshotTakenEventPayload {
  streamId: string;
  snapshotId: string;
  trigger: SnapshotTrigger;
  effortId: string | null;
  threadId: string | null;
}

/** Fires after a snapshot take recorded something new for `streamId`
 *  (a new snapshot, or the current one re-stamped with a new HEAD). */
export function subscribeSnapshotEvents(
  streamId: string,
  fn: (payload: SnapshotTakenEventPayload) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "snapshotTaken") return;
    const eventStreamId = event.streamId as string;
    if (eventStreamId !== streamId) return;
    fn({
      streamId: eventStreamId,
      snapshotId: String(event.snapshotId),
      trigger: event.trigger as SnapshotTrigger,
      effortId: (event.effortId as string | null | undefined) ?? null,
      threadId: (event.threadId as string | null | undefined) ?? null,
    });
  });
}

export async function readWorkspaceFile(streamId: string, path: string): Promise<WorkspaceFile> {
  return unwrap(await commands.readWorkspaceFile(streamId || null, path));
}

/** Where a `symbol:` ref points in `streamId` — the current symbol, or,
 *  for a ref pinned `@snap:`/`@git:`, where it was in that version (with
 *  `versionGone` when the pin is gone and this is the current one); null
 *  for a dead link. */
export async function resolveSymbol(streamId: string, ref: string): Promise<SymbolLocation | null> {
  return unwrap(await commands.resolveSymbol(streamId, ref));
}

/** Where a file is read: its filesystem (none: `oxplow:local`, the
 *  stream's worktree) and the revision on it, the filesystem's own word
 *  (locally `working`, `snap:<id>`, `git:<rev>`; none: its default, the
 *  working tree). */
export interface FileAt {
  filesystem?: string | null;
  rev?: Revision | null;
}

/** `path` on a filesystem at a revision, as text; null when it isn't
 *  there. */
export async function readAt(streamId: string, path: string, at: FileAt = {}): Promise<string | null> {
  return unwrap(await commands.readAt(streamId || null, path, at.filesystem ?? null, at.rev ?? null));
}

/** What `path` is on a filesystem at a revision (kind, size, whether it
 *  is text, language); null when it isn't there. */
export async function statFile(streamId: string, path: string, at: FileAt = {}): Promise<FsStat | null> {
  return unwrap(await commands.statFile(streamId || null, path, at.filesystem ?? null, at.rev ?? null));
}

/** Save a file of the stream's worktree: `oxplow.file.save`. */
export async function writeWorkspaceFile(
  streamId: string,
  path: string,
  content: string,
): Promise<{ path: string; bytes: number }> {
  const out = await runCommand("oxplow.file.save", { stream: streamId, path, content });
  return out.result as { path: string; bytes: number };
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

// ---- The Assistant (`oxplow_app::assistant`) ------------------------------
// A short-lived agent in oxplow's own panel. `assistantPrompt` is the
// prompt box's Enter and nothing else. A session lives in memory: gone on
// restart.

/** Open a session in `stream` (and `thread`) on what was asked about. */
export async function assistantOpen(
  stream: string,
  thread: string | null,
  context: AssistantContext | null,
): Promise<string> {
  return unwrap(await commands.assistantOpen(stream, thread, context)).id;
}

/** Send what the person typed. Fails `BUSY` while a turn runs and
 *  `NO_MODEL` with no model bound. */
export async function assistantPrompt(id: string, text: string): Promise<void> {
  unwrap(await commands.assistantPrompt(id, text));
}

/** Stop the running turn. */
export async function assistantCancel(id: string): Promise<void> {
  unwrap(await commands.assistantCancel(id));
}

/** Change the session's settings; a field left out stays as it is.
 *  `model: null` goes back to the `assistant` role's model. */
export async function assistantSet(
  id: string,
  change: { model?: AssistantModel | null; allowChanges?: boolean },
): Promise<void> {
  const model = change.model === undefined ? null : change.model === null ? "default" : change.model;
  unwrap(await commands.assistantSet(id, model, change.allowChanges ?? null));
}

/** Answer approval card `ask` (its `requestId`). */
export async function assistantAnswer(id: string, ask: string, allow: boolean): Promise<void> {
  unwrap(await commands.assistantAnswer(id, ask, allow));
}

/** The session, with the transcript items changed after `since`. */
export async function assistantSnapshot(id: string, since: number): Promise<AssistantSnapshot> {
  return unwrap(await commands.assistantSnapshot(id, since));
}

/** Continue in thread `toThread` (`thr3`): hand it the condensed transcript. */
export async function assistantHandoff(id: string, toThread: string): Promise<void> {
  unwrap(await commands.assistantHandoff(id, toThread));
}

/** Close the session, stopping its turn. */
export async function assistantClose(id: string): Promise<void> {
  unwrap(await commands.assistantClose(id));
}

/** Call `onChange(seq)` whenever session `id` changed. */
export function subscribeAssistant(id: string, onChange: (seq: number) => void): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind === "assistantChanged" && event.id === id) onChange(event.seq);
  });
}

export function subscribeWorkspaceContext(
  onEvent: (next: WorkspaceContext) => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "workspaceContextChanged") return;
    onEvent({ vcsEnabled: Boolean(event.vcsEnabled) });
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

export type AgentStatus = "working" | "waiting" | "stalled" | "awaiting";

export interface AgentStatusEntry {
  streamId: string;
  threadId: string;
  /** The agent session (`ses3`); `null` for activity no session claims. */
  sessionId: string | null;
  status: AgentStatus;
  /// What the agent is waiting on (its question, a permission), present only when `status` is
  /// "awaiting". Surfaced as the rail dot's tooltip so you can see what
  /// a (possibly different) thread is asking without switching to it.
  question?: string;
}

/// Collapse the backend `AgentStatusState` enum to the dot's alphabet.
/// "running" → working; "stalled" (derived when a Running hook log
/// goes silent past the stall threshold — the agent died without ever
/// emitting a Stop hook) stays distinct so the dot can render it as a
/// failure rather than ordinary waiting; "awaiting_user" (the agent
/// asked the person something) stays distinct so the dot can
/// render "waiting on you"; everything else (idle / stopped / error) →
/// waiting.
export function collapseAgentStatusState(raw: string | undefined): AgentStatus {
  if (raw === "running") return "working";
  if (raw === "stalled") return "stalled";
  if (raw === "awaiting_user") return "awaiting";
  return "waiting";
}

/// The Interrupt hook the agent terminal's Escape synthesizes for agent
/// session `sessionId` (on `threadId`). Claude Code cancels the in-flight
/// turn on Escape but does not emit a Stop/Interrupt hook itself, so the
/// working-dot would stay Running until the next user prompt. Posting this
/// closes the session's open agent_turn and flips its derived status back
/// to Idle immediately.
export function userInterruptEnvelope(sessionId: string, threadId: string, streamId: string | null): HookEnvelope {
  return {
    kind: "interrupt",
    thread_id: threadId,
    stream_id: streamId,
    agent_session_id: sessionId,
    session_id: null,
    payload_json: JSON.stringify({ source: "user-escape" }),
    prompt: null,
    decision: null,
  };
}

export async function recordUserInterrupt(sessionId: string, threadId: string, streamId: string | null): Promise<void> {
  unwrap(await commands.ingestHookEvent(userInterruptEnvelope(sessionId, threadId, streamId)));
}

export async function listAgentStatuses(_streamId?: string): Promise<AgentStatusEntry[]> {
  // The Rust binding returns the raw `AgentStatus` row
  // ({ thread_id, state: "idle"|"running"|..., detail }). The
  // renderer only cares about the dot's narrow alphabet, so collapse
  // the AgentStatusState enum here (see collapseAgentStatusState).
  // Without this transform the consumer reads `entry.threadId` and
  // `entry.status` off raw rows that have neither field, so the dot
  // never leaves its waiting fallback.
  const rows = unwrap(await commands.listAgentStatuses());
  return rows.map((row) => {
    const status = collapseAgentStatusState(row.state);
    return {
      streamId: row.stream_id,
      threadId: row.thread_id,
      sessionId: row.agent_session_id,
      // detail is what it waits on only while awaiting; other
      // states reuse detail for markers ("boot"/"interrupt") the dot
      // shouldn't surface, so scope the tooltip to the awaiting state.
      status,
      question: status === "awaiting" ? (row.detail ?? undefined) : undefined,
    };
  });
}

export interface PageVisitInputApi {
  refKind: string;
  refId: string;
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


/** Re-read visits whenever one is recorded or forgotten (`v_page_visit`). */
export function subscribePageVisitEvents(onEvent: () => void): () => void {
  return onDatasetsChanged(["v_page_visit"], onEvent);
}

/** Drop every visit row for a given page reference. Used when a page
 *  is deleted (real persistent or virtual, e.g. an op-error entry) so
 *  it disappears from rail history. Generic — not tied to any one
 *  page kind. */
export async function forgetPage(refKind: string, refId: string): Promise<void> {
  unwrap(await commands.forgetPage(refKind, refId));
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
    const detail = event.detail as string | null | undefined;
    const eventStream = event.streamId as string | undefined;
    if (!eventStream || (streamId !== "all" && eventStream !== streamId)) return;
    onEvent({
      streamId: eventStream,
      threadId,
      sessionId: (event.agentSessionId as string | null | undefined) ?? null,
      status,
      question: status === "awaiting" ? (detail ?? undefined) : undefined,
    });
  });
}

export async function probeDaemon(): Promise<boolean> {
  try {
    unwrap(await commands.ping());
    return true;
  } catch (e) {
    // A refused token is the daemon answering: it's there, and the
    // banner says what's wrong (tsk971).
    return e instanceof IpcCallError && e.code === "UNAUTHORIZED";
  }
}

/** A tool call's stored input or output (by the hash in its event
 *  payload), or null once retention removed it. */
export async function readEventContent(
  eventId: string,
  body: "input" | "output",
): Promise<import("./tauri-bridge/generated/bindings.js").EventBody | null> {
  return unwrap(await commands.readEventContent(eventId, body));
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

/** Install a language server (a Mason package): `oxplow.lsp.install_server`. */
export async function installLspPackage(packageName: string): Promise<InstalledLspPackage> {
  // The person's click (or the install prompt) is the confirmation.
  return (await runCommand("oxplow.lsp.install_server", { package: packageName }, true))
    .result as InstalledLspPackage;
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
  await runCommand("oxplow.lsp.restart_server", { stream: streamId, language: languageId });
}

export async function removeLspPackage(packageName: string): Promise<void> {
  await runCommand("oxplow.lsp.remove_server", { package: packageName }, true);
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

/** Pages pointing AT the URI `target`. */
export async function listBacklinks(target: string, limit: number | null = null): Promise<BacklinkEdge[]> {
  return unwrap(await commands.listBacklinks(target, limit));
}

// ----------------------------------------------------------------------
// The event log's dead-letter queue (.context/data-model.md "event_log").
// Read through `v_event_dead_letter` (delivery.ts); a person retries or
// discards here.
// ----------------------------------------------------------------------

/** Pages this source points AT. Inverse of `listBacklinks`. */
export async function listPageOutbound(source: string, limit: number | null = null): Promise<BacklinkEdge[]> {
  return unwrap(await commands.listOutbound(source, limit));
}

