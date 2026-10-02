import { commands } from "./tauri-bridge/generated/bindings.js";
import type { OpOutcome, ProviderEffect, Reads, SnapshotTrigger } from "./tauri-bridge/generated/bindings.js";
import { listen, onRemoteReconnect, triggerRemoteResync } from "./tauri-bridge/transport.js";

export { onRemoteReconnect, triggerRemoteResync };
import { EVENT_CHANNELS } from "./tauri-bridge/channels.js";
import type { OxplowEvent } from "./api-types.js";
import { latestWins } from "./latestWins.js";
import {
  METRIC_CATALOG_SQL,
  METRIC_SPECS_SQL,
  catalogEntries,
  metricSeriesSql,
  metricSpecs as metricSpecRows,
  seriesPoints as seriesPointRows,
  type MetricCatalogEntry,
  type MetricSpec,
  type SeriesPoint,
} from "./metricsSql.js";
import { normalizeSnapshotId } from "./effort-snapshot.js";
import { taskIdOfWorkItemRef, workItemRef } from "./workItemRef.js";
import { IpcCallError, ipcErrorCode, ipcErrorMessage } from "./ipc-error.js";
import type {
  AiSettings,
  CatalogPrompt,
  EffectiveSetting,
  PanelPlacement,
  ChangeRow,
  ChangeTarget,
  CheckReport,
  CommentIntent,
  CommentMessage,
  CommentStatus,
  CommentThread,
  DiffEntry,
  Extension,
  ExtensionReview,
  Lens,
  LensRun,
  LensViz,
  LensSpec,
  ProviderConfig,
  RecentProjectView,
  Role,
  RoleBinding,
  CollectorListing,
  CollectorRunReport,
  DataEntity,
  CommandOutcome,
  CommandSpec,
  FormStart,
  AcpAgentListing,
  AcpEvent,
  AcpSnapshot,
  AcpStatus,
  ContextUsage,
  PermissionOption,
  PlanEntry,
  ToolCall as AcpToolCall,
  ToolDiff,
  TranscriptItem,
  ProgramKind,
  ProjectProgram,
  ProviderInstanceView,
  SearchHit,
  SqlCell,
  SqlQueryResult,
} from "./tauri-bridge/generated/bindings.js";

export type { AiSettings, ProviderConfig, Role, RoleBinding };
export type {
  AcpEvent,
  AcpSnapshot,
  AcpStatus,
  AcpToolCall,
  ContextUsage,
  PermissionOption,
  PlanEntry,
  ToolDiff,
  TranscriptItem,
};
export type { DiffEntry };
export type { DataEntity, Extension, ExtensionReview, Lens, LensRun, LensViz, LensSpec, SearchHit, CollectorListing, CollectorRunReport, SqlCell, SqlQueryResult };
export type { ProviderInstanceView };

/// Convert the tauri-specta {status, data|error} envelope into a
/// plain promise return. Errors are usually IpcError objects with
/// message/code, but arg-deserialization failures and panics arrive as
/// a plain string — `ipcErrorMessage` surfaces the real reason verbatim
/// instead of collapsing to a generic "ipc error" (see ipc-error.ts).
function unwrap<T>(result: { status: "ok"; data: T } | { status: "error"; error: unknown }): T {
  if (result.status === "ok") return result.data;
  throw new IpcCallError(ipcErrorMessage(result.error), ipcErrorCode(result.error));
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
// RemoteBranchEntry, BlameLine, …). The api-types
// camelCase legacy definitions were drifting from runtime shape
// and only existed because the original Electron build wrapped
// them in adapters; nothing converts shape today.
// Bindings shapes for the types whose call sites have been
// migrated. Adding more is a per-call-site refactor: each consumer
// has to be updated to the new field names. Types not on this list
// stay on the api-types camelCase legacy shape until their consumers
// are migrated.
export type {
  OpOutcome,
  RemoteBranchEntry,
  MergeReadiness,
} from "./tauri-bridge/index.js";
// The remaining legacy types still come from api-types because
// their consumers read fields that don't exist on the bindings
// shape yet (e.g. GitLogResult.currentBranch / branchHeads / tags,
// RemoteBranchEntry.remote / branch / lastCommitDate, GitWorktreeEntry
// camelCase aliases, ChangeScopes' BranchChangeEntry.status — bindings
// expose .change).
// Migrating each one is per-call-site work; until then the shape
// the runtime hands the renderer is the bindings shape but the
// renderer's TypeScript believes it's the legacy shape.
export type {
  ChangeScopes,
  TextSearchHit,
  RefOption,
  GroupedGitRefs,
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

// Tasks are read from the models by the work-item data layer
// (`workItems.ts`, P6.E1b), which owns their shape.
export type { Task, TaskStatus, TaskPriority, ThreadWorkState, BacklogState, FinishedEntry } from "./workItems.js";

export interface TaskNote {
  id: string;
  task_id: string;
  body: string;
  author: string;
  created_at: string;
}


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
   *  provider's `work_item:linear:ENG-12`). */
  work_item: string;
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

import type { Followup as ThreadFollowup } from "./tauri-bridge/index.js";
export type { ThreadFollowup };

export const BACKLOG_SCOPE = "__backlog__";

export interface BranchRef {
  kind: "local" | "remote";
  name: string;
  ref: string;
  remote?: string;
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

import type { Revision } from "./revision.js";
import type {
  BlameLine,
  Divergence,
  FileStatus,
  HeadInfo,
  InProgressOp,
  RevisionDetail,
  RevisionInfo,
  VcsWorkspace,
  StatusEntry,
  WorkspaceEntry,
  WorkspaceIndexedFile,
  WorkspaceStatus,
} from "./tauri-bridge/generated/bindings.js";
export type {
  BlameLine,
  Divergence,
  FileStatus,
  HeadInfo,
  InProgressOp,
  RevisionInfo,
  VcsWorkspace,
  RevisionDetail,
  StatusEntry,
  WorkspaceEntry,
  WorkspaceIndexedFile,
  WorkspaceStatus,
};

// ---- The stream's version control, neutral (`.context/vcs.md`) ----

/** Where the stream's workspace is: its head revision and branch. */
export async function vcsHead(streamId: string): Promise<HeadInfo> {
  return unwrap(await commands.vcsHead(streamId || null));
}

/** The workspace's changes against its head, plus any paused operation. */
export async function vcsStatus(streamId: string): Promise<WorkspaceStatus> {
  return unwrap(await commands.vcsStatus(streamId || null));
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

/** A revision's message and changed files; null when the workspace
 *  doesn't have it. */
export async function vcsRevision(
  streamId: string,
  revision: Revision,
): Promise<RevisionDetail | null> {
  return unwrap(await commands.vcsRevision(streamId || null, revision));
}

/** How far `head` and `base` have diverged, and whether `head` would
 *  merge cleanly (live; a stream's history reads `v_commit`). */
export async function vcsDivergence(
  streamId: string,
  base: Revision,
  head: Revision,
): Promise<Divergence> {
  return unwrap(await commands.vcsDivergence(streamId || null, base, head));
}

/** Revisions on `head` that `base` lacks, newest first. */
export async function vcsRevisionsBetween(
  streamId: string,
  base: Revision,
  head: Revision,
  limit = 200,
): Promise<RevisionInfo[]> {
  return unwrap(await commands.vcsRevisionsBetween(streamId || null, base, head, limit));
}

/** Revisions that changed `path`, newest first. */
export async function vcsFileHistory(
  streamId: string,
  path: string,
  limit = 50,
): Promise<RevisionInfo[]> {
  return unwrap(await commands.vcsFileHistory(streamId || null, path, limit));
}

/** Working copies of the repository no stream uses yet. */
export async function vcsListAdoptableWorkspaces(): Promise<VcsWorkspace[]> {
  return unwrap(await commands.vcsListAdoptableWorkspaces());
}

/** Where the histories of `a` and `b` fork. */
export async function vcsMergeBase(
  streamId: string,
  a: Revision,
  b: Revision,
): Promise<Revision | null> {
  return unwrap(await commands.vcsMergeBase(streamId || null, a, b));
}

/** How many paths of each kind a status holds. Renames count as
 *  modifications only where a caller folds them. */
export interface StatusCounts {
  added: number;
  modified: number;
  deleted: number;
  renamed: number;
  untracked: number;
  conflicted: number;
  total: number;
}

export function countStatus(status: WorkspaceStatus): StatusCounts {
  const counts: StatusCounts = {
    added: 0,
    modified: 0,
    deleted: 0,
    renamed: 0,
    untracked: 0,
    conflicted: 0,
    total: 0,
  };
  for (const e of status.entries) {
    counts[e.status] += 1;
    counts.total += 1;
  }
  return counts;
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
/// everything. `kinds` optionally restricts to task|comment|note|wiki|file.
export async function searchSite(
  query: string,
  streamId: string | null,
  kinds: string[] | null = null,
  limit = 50,
): Promise<SearchHit[]> {
  return unwrap(await commands.search(query, streamId, kinds, limit));
}

/// Run one read-only `SELECT`/`WITH` over the semantic layer's `v_*`
/// views (see `.context/semantic-layer.md`). Positional `params` bind
/// `?1`, `?2`, …; `limit` caps rows (default 500).
/** One read over the published models. `raw` (the explorer only) reads
 *  physical tables too; the result's `reads` says what it read. */
export async function querySql(
  sql: string,
  params: SqlCell[] = [],
  limit: number | null = null,
  raw = false,
): Promise<SqlQueryResult> {
  return unwrap(await commands.querySql(sql, params, limit, raw));
}

/// Project extensions (and their lenses) in the stream's worktree, with
/// per-extension load errors. See `.context/extensions.md`.
export async function listExtensions(streamId: string | null): Promise<Extension[]> {
  return unwrap(await commands.listExtensions(streamId));
}

/// One lens by `<extension>/<slug>`.
export async function getLens(id: string, streamId: string | null): Promise<Lens> {
  return unwrap(await commands.getLens(id, streamId));
}

/// Run a lens with param overrides (the rest use defaults).
export async function runLens(
  id: string,
  params: Record<string, SqlCell>,
  streamId: string | null,
): Promise<LensRun> {
  return unwrap(await commands.runLens(id, params, streamId));
}

/// Run one of a lens's declared buttons (never approves an exec source).
/** A person presses a lens's action: its command runs as the lens,
 *  acting for them. Throws `NEEDS_CONFIRMATION` (see `needsConfirmation`)
 *  when the command asks; call again with `confirmed`. */
export async function runLensAction(
  id: string,
  action: string,
  params: Record<string, SqlCell>,
  row: Record<string, SqlCell> | null,
  streamId: string | null,
  confirmed: boolean,
): Promise<CommandOutcome> {
  return unwrap(await commands.runLensAction(id, action, params, row, streamId, confirmed));
}

/** A command's spec: what a form renders from its `input_schema`, and
 *  what a confirmation says. */
export async function getCommand(name: string): Promise<CommandSpec> {
  return unwrap(await commands.getCommand(name));
}

/** A form lens's command and the values its fields start from. */
export async function lensForm(id: string, params: Record<string, SqlCell>, streamId: string | null): Promise<FormStart> {
  return unwrap(await commands.lensForm(id, params, streamId));
}

/** Submit a form lens: its command runs as the lens, acting for the
 *  person. Throws `NEEDS_CONFIRMATION` when the command asks. */
export async function submitLensForm(
  id: string,
  input: Record<string, unknown>,
  params: Record<string, SqlCell>,
  streamId: string | null,
  confirmed: boolean,
): Promise<CommandOutcome> {
  return unwrap(await commands.submitLensForm(id, input, params, streamId, confirmed));
}

/** A custom component's frame reads one of its declared lenses, while the
 *  person looks at lens `id` (P6b.D2). */
export async function runComponentQuery(
  id: string,
  asset: string,
  params: Record<string, SqlCell>,
  streamId: string | null,
): Promise<LensRun> {
  return unwrap(await commands.runComponentQuery(id, asset, params, streamId));
}

/** A custom component's frame invokes one of its declared commands, as the
 *  lens acting for the person; `confirmed` only after the host asked. */
export async function invokeComponentCommand(
  id: string,
  command: string,
  input: unknown,
  streamId: string | null,
  confirmed: boolean,
): Promise<CommandOutcome> {
  return unwrap(await commands.invokeComponentCommand(id, command, input as never, streamId, confirmed));
}

/** What the person can ask: every capability's questions and the stream's
 *  enabled extensions' prompts (the catalog; contextual suggestions). */
export async function promptCatalog(streamId: string | null): Promise<CatalogPrompt[]> {
  return unwrap(await commands.promptCatalog(streamId));
}

/** Every setting with its value and where it comes from (P6.H1). */
export async function effectiveConfig(): Promise<EffectiveSetting[]> {
  return unwrap(await commands.effectiveConfig());
}

/** The person's left-nav layout: each panel's order, and whether it's
 *  hidden or collapsed (P6.G1). */
export async function getPanelLayout(): Promise<PanelPlacement[]> {
  return unwrap(await commands.getPanelLayout());
}

export async function setPanelLayout(layout: PanelPlacement[]): Promise<void> {
  unwrap(await commands.setPanelLayout(layout));
}

/** Run an answer an agent showed in a thread (`answer:<id>`). */
export async function runAnswer(answer: string): Promise<LensRun> {
  return unwrap(await commands.runAnswer(answer));
}

/** A lens's text rendering — what Copy puts on the clipboard. */
export async function lensText(id: string, params: Record<string, SqlCell>, streamId: string | null): Promise<string> {
  return unwrap(await commands.lensText(id, params, streamId));
}

/// Load an extension and dry-run every lens, returning all problems.
export async function validateExtension(name: string, streamId: string | null): Promise<CheckReport> {
  return unwrap(await commands.validateExtension(name, streamId));
}

/// Tell the backend which page the human has open in a thread (for the
/// agent's `get_open_page`). `pageId` null = nothing open.
export async function reportOpenPage(
  threadId: string,
  pageId: string | null,
  kind: string | null,
  detailJson: string | null,
): Promise<void> {
  unwrap(await commands.reportOpenPage(threadId, pageId, kind, detailJson));
}

/// Turn an extension on or off for the project (`extensions.disabled` in
/// `.oxplow/project.yaml`). Returns the updated extension list.
export async function setExtensionEnabled(name: string, enabled: boolean): Promise<Extension[]> {
  return unwrap(await commands.setExtensionEnabled(name, enabled));
}

/// Extension-declared collectors with their last run and consent.
export async function listCollectors(): Promise<CollectorListing[]> {
  return unwrap(await commands.listCollectors());
}

/** A person approves an exec collector at the listing's `version` they
 *  reviewed (refused if it changed since). */
export async function approveCollector(owner: string, id: string, version: string): Promise<void> {
  unwrap(await commands.approveCollector(owner, id, version));
}

/** Run a collector now (`collector.sync`); it never approves. */
export async function syncCollector(owner: string, id: string): Promise<CollectorRunReport> {
  return (await runCommand("collector.sync", { owner, id })).result as CollectorRunReport;
}

/// Analyze a change (a commit, an effort, or a stream's working tree) if
/// needed and return its `v_change` row. See `.context/semantic-layer.md`.
export async function ensureChange(target: ChangeTarget): Promise<ChangeRow> {
  return unwrap(await commands.ensureChange(target));
}

/// Set (or clear with null) a credential an extension's collector or
/// provider declares. The value goes to the OS keychain and is never
/// returned.
export async function setCredential(extension: string, name: string, value: string | null): Promise<void> {
  unwrap(await commands.setCredential(extension, name, value));
}

/// Settings → AI: providers (whether each has a key, never the key) and
/// every role's assignment. See `.context/ai-providers.md`.
export async function aiSettings(): Promise<AiSettings> {
  return unwrap(await commands.aiSettings());
}

/// Add or replace a provider. A non-empty `key` goes to the OS keychain;
/// null keeps the stored one.
export async function saveAiProvider(provider: ProviderConfig, key: string | null): Promise<AiSettings> {
  return unwrap(await commands.saveAiProvider(provider, key));
}

/// Remove a provider and its key (refused while a role uses it).
export async function removeAiProvider(id: string): Promise<AiSettings> {
  return unwrap(await commands.removeAiProvider(id));
}

/// Assign a role to a provider + model, or unassign it with null.
export async function setAiRole(role: Role, binding: RoleBinding | null): Promise<AiSettings> {
  return unwrap(await commands.setAiRole(role, binding));
}

/// One small call to check a provider's key, URL and model; returns the reply.
export async function testAiProvider(id: string, model: string): Promise<string> {
  return unwrap(await commands.testAiProvider(id, model));
}

/// Save a query as a new lens file (`oxplow/extensions/<extension>/lenses/<slug>.yaml`).
export async function saveLens(
  extension: string,
  slug: string,
  lens: LensSpec,
  streamId: string | null,
): Promise<Lens> {
  return unwrap(await commands.saveLens(extension, slug, lens, streamId));
}

/// What installing (`gitUrl`) or updating (`name`) an extension would
/// bring in, installing nothing: shown for the person to confirm.
export async function reviewExtension(
  target: { gitUrl: string; gitRef?: string | null } | { name: string },
  streamId: string | null,
): Promise<ExtensionReview> {
  return "name" in target
    ? unwrap(await commands.reviewExtension(null, null, target.name, streamId))
    : unwrap(await commands.reviewExtension(target.gitUrl, target.gitRef ?? null, null, streamId));
}

/// Install an extension from a git repo into the stream's worktree, at the
/// commit the person reviewed.
export async function installExtension(
  gitUrl: string,
  gitRef: string | null,
  reviewedSha: string,
  streamId: string | null,
): Promise<Extension> {
  return unwrap(await commands.installExtension(gitUrl, gitRef, reviewedSha, streamId));
}

/// Re-install a git-installed extension from its recorded source, at the
/// commit the person reviewed.
export async function updateExtension(
  name: string,
  reviewedSha: string,
  streamId: string | null,
): Promise<Extension> {
  return unwrap(await commands.updateExtension(name, reviewedSha, streamId));
}

/// Programs the project's config would run, and whether each is approved.
export async function listProjectPrograms(): Promise<ProjectProgram[]> {
  return unwrap(await commands.listProjectPrograms());
}

/** What approving provider `instance` as it is on disk would change
 *  against what was approved last (P6b.E3). */
export async function providerDeclarationEffects(instance: string): Promise<ProviderEffect> {
  return unwrap(await commands.providerDeclarationEffects(instance));
}

/// A person approves one of the project's programs (Settings → Data).
/// `version` is the listing's, the one the person reviewed; a program that
/// changed since is refused.
export async function approveProjectProgram(
  kind: ProgramKind,
  name: string,
  version: string,
): Promise<ProjectProgram[]> {
  return unwrap(await commands.approveProjectProgram(kind, name, version));
}

/// Extension providers' instances on this machine, with health
/// (Settings → Integrations).
export async function listProviderInstances(): Promise<ProviderInstanceView[]> {
  return unwrap(await commands.listProviderInstances());
}

/// Check an instance against `config` without enabling or saving
/// anything; the outcome is the returned view's state.
export async function checkProviderInstance(instance: string, config: unknown): Promise<ProviderInstanceView> {
  return unwrap(await commands.checkProviderInstance(instance, config));
}

/// A person saves an instance's config and enables or disables it.
/// Enabling checks first; an unapproved or unconfigured instance is
/// refused and nothing is written.
export async function setProviderInstance(
  instance: string,
  enabled: boolean,
  config: unknown,
): Promise<ProviderInstanceView[]> {
  return unwrap(await commands.setProviderInstance(instance, enabled, config));
}

/// The ACP agents this project can run (the new-thread picker).
export async function listAcpAgents(): Promise<AcpAgentListing[]> {
  return unwrap(await commands.listAcpAgents());
}

// ---- ACP sessions (tsk281) --------------------------------------------
// A structured agent conversation instead of a terminal. `acpPrompt` is
// the prompt box's Enter and nothing else: oxplow never sends an agent
// input on its own (guarded by no-agent-input-automation.test.ts).

/** Start the thread's ACP agent (or reattach) and return its session. */
export async function acpOpenSession(threadId: string): Promise<AcpSnapshot> {
  return unwrap(await commands.acpOpenSession(threadId));
}

/** Send what the person typed. Only `AcpPromptBox` calls this. */
export async function acpPrompt(threadId: string, text: string): Promise<void> {
  unwrap(await commands.acpPrompt(threadId, text));
}

export async function acpCancel(threadId: string): Promise<void> {
  unwrap(await commands.acpCancel(threadId));
}

/** Answer a permission card; `optionId: null` cancels it. */
export async function acpRespondPermission(
  threadId: string,
  requestId: string,
  optionId: string | null,
): Promise<void> {
  unwrap(await commands.acpRespondPermission(threadId, requestId, optionId));
}

/** The session plus items changed after `sinceSeq`; null when none is open. */
export async function acpTranscript(threadId: string, sinceSeq: number): Promise<AcpSnapshot | null> {
  return unwrap(await commands.acpTranscript(threadId, sinceSeq));
}

export async function acpDismissDirective(threadId: string): Promise<void> {
  unwrap(await commands.acpDismissDirective(threadId));
}

export async function acpCloseSession(threadId: string): Promise<void> {
  unwrap(await commands.acpCloseSession(threadId));
}

export function subscribeAcpEvents(listener: (event: AcpEvent) => void): () => void {
  let stopped = false;
  // `listen` rejects when no transport is mounted (bun tests).
  const unlistenPromise = listen(EVENT_CHANNELS.acp, (e) => {
    if (stopped) return;
    listener(e.payload as AcpEvent);
  }).catch(() => null);
  return () => {
    stopped = true;
    void unlistenPromise.then((u) => u?.());
  };
}

/// Settings → Data: every published model with its row count, and the
/// entities extensions declare that haven't synced.
export async function listDataEntities(): Promise<DataEntity[]> {
  return unwrap(await commands.listDataEntities());
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
 * resolves with the final `BackgroundTask` (status, error, and
 * `result` payload — the command's `OpOutcome`). Pattern:
 *
 *     const { taskId, awaitDone } = await gitRebase(...);
 *     // mark UI pending using taskId / a label
 *     const task = await awaitDone;
 *     // task.result is the OpOutcome
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

export async function vcsMerge(streamId: string, rev: string, confirmed: boolean): Promise<GitOpKickoff> {
  return runAsBackgroundTask(`Merge ${rev}`, "vcs", `merge ${rev}`, () =>
    runVcs("vcs.merge", { stream: streamId, rev }, confirmed),
  );
}

export async function gitRebase(streamId: string, onto: string, confirmed: boolean): Promise<GitOpKickoff> {
  return runAsBackgroundTask(`Rebase onto ${onto}`, "vcs", `rebase ${onto}`, () =>
    runVcs("git.rebase", { stream: streamId, rev: onto }, confirmed),
  );
}

export async function gitCherryPick(streamId: string, rev: string): Promise<GitOpKickoff> {
  const short = rev.slice(0, 7);
  return runAsBackgroundTask(`Cherry-pick ${short}`, "vcs", `cherry-pick ${short}`, () =>
    runVcs("git.cherry_pick", { stream: streamId, rev }),
  );
}

export async function gitRevert(streamId: string, rev: string, confirmed: boolean): Promise<GitOpKickoff> {
  const short = rev.slice(0, 7);
  return runAsBackgroundTask(`Revert ${short}`, "vcs", `revert ${short}`, () =>
    runVcs("git.revert", { stream: streamId, rev }, confirmed),
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

/** Switch the stream's workspace to `branch` (`create` makes it at the
 *  head first). The branch reconciler records it on the stream. */
export async function vcsCheckoutBranch(streamId: string, branch: string, create = false): Promise<OpOutcome> {
  return runVcs("vcs.checkout_branch", { stream: streamId, name: branch, create });
}

export async function vcsRenameBranch(streamId: string, from: string, to: string): Promise<OpOutcome> {
  return runVcs("vcs.rename_branch", { stream: streamId, from, to });
}

export async function vcsDeleteBranch(
  streamId: string,
  name: string,
  force: boolean,
  confirmed: boolean,
): Promise<OpOutcome> {
  return runVcs("vcs.delete_branch", { stream: streamId, name, force }, confirmed);
}

export async function getThreadState(streamId: string): Promise<ThreadState> {
  return unwrap(await commands.getThreadState(streamId)) as unknown as ThreadState;
}

/** A thread command's input names threads and streams by ref. */
const threadRef = (id: string) => `thread:${id}`;
const streamRef = (id: string) => `stream:${id}`;

export async function createThread(
  streamId: string,
  title: string,
  agent?: AgentKind,
  acpAgent?: string | null,
): Promise<ThreadState> {
  await runCommand("thread.create", {
    stream: streamRef(streamId),
    title,
    ...(agent ? { agent } : {}),
    ...(acpAgent ? { acp_agent: acpAgent } : {}),
  });
  return getThreadState(streamId);
}

export async function reorderThreads(streamId: string, orderedThreadIds: string[]): Promise<void> {
  await runCommand("thread.reorder", {
    stream: streamRef(streamId),
    order: orderedThreadIds.map(threadRef),
  });
}

export async function reorderStreams(orderedStreamIds: string[]): Promise<void> {
  unwrap(await commands.reorderStreams(orderedStreamIds));
}

export async function selectThread(streamId: string, threadId: string): Promise<ThreadState> {
  unwrap(await commands.selectThread({ streamId, threadId }));
  return getThreadState(streamId);
}

export async function promoteThread(streamId: string, threadId: string): Promise<ThreadState> {
  await runCommand("thread.promote", { thread: threadRef(threadId) });
  return getThreadState(streamId);
}

export async function closeThread(streamId: string, threadId: string): Promise<ThreadState> {
  await runCommand("thread.close", { thread: threadRef(threadId) });
  return getThreadState(streamId);
}

export async function reopenThread(streamId: string, threadId: string): Promise<ThreadState> {
  await runCommand("thread.reopen", { thread: threadRef(threadId) });
  return getThreadState(streamId);
}

export async function listClosedThreads(streamId: string): Promise<Thread[]> {
  return unwrap(await commands.listClosedThreads(streamId));
}

export async function renameThread(_streamId: string, threadId: string, title: string): Promise<Thread> {
  return (await runCommand("thread.rename", { thread: threadRef(threadId), title })).result as Thread;
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
  await runCommand("thread.set_prompt", { thread: threadRef(threadId), ...(prompt ? { prompt } : {}) });
  return [];
}

export async function getChangeScopes(
  streamId: string,
): Promise<import("./api-types.js").ChangeScopes> {
  const raw = unwrap(await commands.gitChangeScopes(streamId));
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

/** Throw away the workspace's changes to `paths` — destructive, so the
 *  person has confirmed. */
export async function vcsDiscard(streamId: string, paths: string[], confirmed: boolean): Promise<OpOutcome> {
  return runVcs("vcs.discard", { stream: streamId, paths }, confirmed);
}

export async function vcsStage(streamId: string, paths: string[]): Promise<OpOutcome> {
  return runVcs("vcs.stage", { stream: streamId, paths });
}

/** Append a pattern to the workspace's `.gitignore`. */
export async function gitIgnore(streamId: string, entry: string): Promise<OpOutcome> {
  return runVcs("git.ignore", { stream: streamId, entry });
}

/** A remote branch to push to or pull from; omitted = the upstream. */
export interface RemoteBranchTarget {
  remote: string;
  branch: string;
}

export async function vcsPush(streamId: string, to?: RemoteBranchTarget): Promise<GitOpKickoff> {
  const where = to ? ` ${to.remote} ${to.branch}` : "";
  return runAsBackgroundTask(to ? `Push to ${to.remote}/${to.branch}` : "Push", "vcs", `push${where}`, () =>
    runVcs("vcs.push", { stream: streamId, ...to }),
  );
}

export async function vcsPull(streamId: string, from?: RemoteBranchTarget): Promise<GitOpKickoff> {
  const where = from ? ` ${from.remote} ${from.branch}` : "";
  return runAsBackgroundTask(from ? `Pull ${from.remote}/${from.branch}` : "Pull", "vcs", `pull${where}`, () =>
    runVcs("vcs.pull", { stream: streamId, ...from }),
  );
}

export async function vcsFetch(streamId: string, remote?: string): Promise<GitOpKickoff> {
  return runAsBackgroundTask("Fetch", "vcs", `fetch${remote ? ` ${remote}` : ""}`, () =>
    runVcs("vcs.fetch", { stream: streamId, remote: remote ?? null }),
  );
}

/** Commit the stream's changes; `revision` is the new one (`git:<sha>`). */
export async function vcsCommit(
  streamId: string,
  message: string,
  includeUntracked = true,
): Promise<{ success: boolean; revision: string }> {
  const outcome = await runCommand("vcs.commit", {
    stream: streamId,
    message,
    include_untracked: includeUntracked,
  });
  return outcome.result as { success: boolean; revision: string };
}

export async function listRecentRemoteBranches(
  _streamId: string,
  limit?: number,
): Promise<import("./tauri-bridge/index.js").RemoteBranchEntry[]> {
  return unwrap(await commands.gitListRecentRemoteBranches(limit ?? null));
}

export type UsageRollup = import("./tauri-bridge/generated/bindings.js").UsageRollup;

/** Write a wiki page — the `knowledge.write_page` command (P5.C3): the
 *  row, its links and the file, in one audited run. Every `[[link]]` must
 *  resolve (a dangling one is refused, named). `verifiedRefs` are files
 *  re-checked against the page: their freshness pins move to now. */
export async function writeWikiPage(
  slug: string,
  body: string,
  options?: { verifiedRefs?: string[] },
): Promise<void> {
  await runCommand("knowledge.write_page", {
    slug,
    body,
    verified_refs: options?.verifiedRefs ?? [],
  });
}

/** Delete a wiki page (`knowledge.delete_page`, destructive): the person
 *  has confirmed. */
export async function deleteWikiPage(slug: string, confirmed: boolean): Promise<void> {
  await runCommand("knowledge.delete_page", { slug }, confirmed);
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

export type { MetricSpec, SeriesPoint, MetricCatalogEntry } from "./metricsSql.js";

import type { Dashboard, DashboardWithItems } from "./tauri-bridge/index.js";
export type { Dashboard, DashboardItem, DashboardWithItems } from "./tauri-bridge/index.js";

// User-created dashboards (tsk138) — project-global grids of metric tiles.
export async function listDashboards(): Promise<Dashboard[]> {
  return unwrap(await commands.listDashboards());
}
export async function getDashboard(id: string): Promise<DashboardWithItems | null> {
  return unwrap(await commands.getDashboard(id));
}
export async function createDashboard(title: string): Promise<Dashboard> {
  return unwrap(await commands.createDashboard(title));
}
export async function renameDashboard(id: string, title: string): Promise<void> {
  unwrap(await commands.renameDashboard({ id, title }));
}
export async function deleteDashboard(id: string): Promise<void> {
  unwrap(await commands.deleteDashboard(id));
}
/** Add a tile: `query` (pinned `sql` shown per `display`), `lens` or `text`. */
export async function addDashboardItem(req: {
  dashboardId: string;
  kind: "query" | "lens" | "text";
  sql?: string | null;
  display?: string | null;
  lensId?: string | null;
  optionsJson?: string | null;
}): Promise<string> {
  return unwrap(
    await commands.addDashboardItem({
      dashboardId: req.dashboardId,
      kind: req.kind,
      sql: req.sql ?? null,
      display: req.display ?? null,
      lensId: req.lensId ?? null,
      optionsJson: req.optionsJson ?? null,
    }),
  );
}
export async function updateDashboardItem(id: string, optionsJson: string | null): Promise<void> {
  unwrap(await commands.updateDashboardItem({ id, optionsJson }));
}
export async function removeDashboardItem(id: string): Promise<void> {
  unwrap(await commands.removeDashboardItem(id));
}
export async function reorderDashboardItems(dashboardId: string, order: string[]): Promise<void> {
  unwrap(await commands.reorderDashboardItems({ dashboardId, order }));
}
/** Fires on any dashboard/tile create/edit/reorder/delete (project-global). */
export function subscribeDashboardEvents(fn: () => void): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind === "dashboardsChanged") fn();
  });
}

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

/** One metric's captures, newest first — `metric_grid('capture')` joined to
 *  `v_capture` (P4.7). `groupBy` slices by a dimension (one row per
 *  capture × group); `range` (epoch ms, inclusive) bounds the rows returned. */
export async function listMetricSamples(
  metricKey: string,
  limit?: number,
  groupBy?: string | null,
  range?: { from: number; to: number } | null,
): Promise<MetricRows<SeriesPoint>> {
  const params = range ? [new Date(range.from).toISOString(), new Date(range.to).toISOString()] : [];
  const result = await querySql(metricSeriesSql(metricKey, groupBy, !!range), params, limit ?? 200);
  return { rows: seriesPointRows(result), reads: result.reads };
}

/** Every metric this project can use, with whether it's on — `v_metric_catalog`. */
export async function listMetricCatalog(): Promise<MetricRows<MetricCatalogEntry>> {
  const result = await querySql(METRIC_CATALOG_SQL, [], 10_000);
  return { rows: catalogEntries(result), reads: result.reads };
}

/** Run a command on the bus as the person (P5.A1). `confirmed` says the
 *  person confirmed this exact call; a call that needs one comes back
 *  `NEEDS_CONFIRMATION`, so ask and call again. */
export async function runCommand(name: string, input: unknown, confirmed = false): Promise<CommandOutcome> {
  return unwrap(await commands.runCommand(name, input, confirmed));
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

/** Turn metrics on or off in this project (the `metric.enable` command). */
export async function enableMetrics(keys: string[], enabled: boolean): Promise<void> {
  unwrap(await commands.enableMetrics(keys, enabled));
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
  /** The branch the workspace was on at capture; null for pre-V42 rows, a
   *  detached head, or a directory not under version control. */
  branch: string | null;
  /** What the take that created it recorded (P2.11): the snapshot it grew
   *  from, why it was taken, and whether it ran over its time budget. */
  parentSnapshotId: number | null;
  trigger: SnapshotTrigger | null;
  overBudget: boolean;
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
  const rows = unwrap(await commands.listFileSnapshots(path)) as unknown as Array<{
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
    revision: Revision | null;
    branch: string | null;
    parent_snapshot_id: number | null;
    trigger: SnapshotTrigger | null;
    over_budget: boolean;
  }>;
  return rows.map((r) => ({
    id: r.id,
    streamId: r.stream_id,
    createdAt: r.created_at,
    fileCount: r.file_count,
    revision: r.revision,
    branch: r.branch,
    parentSnapshotId: r.parent_snapshot_id,
    trigger: r.trigger,
    overBudget: r.over_budget,
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

/** What changed from `from` (nothing, when null) to `to` in the
 *  stream's workspace — any two revisions: working tree, snapshots,
 *  commits. Powers the diff views and the changed-files lists. */
export async function diffRevisions(
  streamId: string,
  from: Revision | null,
  to: Revision,
): Promise<DiffEntry[]> {
  return unwrap(await commands.diff(streamId || null, from, to));
}

/** Every file in the stream's workspace at `revision`, sorted. */
export async function filesAt(streamId: string, revision: Revision): Promise<string[]> {
  return unwrap(await commands.filesAt(streamId || null, revision));
}

/** One agent turn by id (`trn<N>`) — its start and end snapshots, for
 *  the turn page's diff. `null` when the id is unknown. */
export async function getAgentTurn(
  turnId: string,
): Promise<{ id: string; startSnapshotId: number | null; snapshotId: number | null } | null> {
  const row = unwrap(await commands.getAgentTurn(turnId)) as unknown as {
    id: string;
    start_snapshot_id: number | null;
    snapshot_id: number | null;
  } | null;
  return row
    ? { id: row.id, startSnapshotId: row.start_snapshot_id, snapshotId: row.snapshot_id }
    : null;
}

/** One effort by id — its snapshot bracket + task id, so the diff view
 *  can resolve `effortDiffRef(effortId)` into (start, end) endpoints.
 *  `null` when the id is unknown. */
export async function getEffort(effortId: string): Promise<OverlappingEffort | null> {
  const row = unwrap(await commands.getEffort(effortId)) as unknown as RawEffort | null;
  return row ? toOverlappingEffort(row) : null;
}

/** Per-effort touched_files list — the canonical authorship list
 *  (LLM-declared via `complete_task` + any subsequent `amend_effort`
 *  corrections).  */
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
  /** The effort's work item ref. */
  workItem: string;
  /** The oxplow task behind `workItem`; `null` for another provider's item. */
  tasksId: string | null;
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
 *  effort CLAIMED (effort_file) vs the `unclaimed` rest (parallel/
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
      work_item: string;
      thread_id: string;
      start_snapshot_id: number | null;
      end_snapshot_id: number | null;
    };
  }>;
  return rows.map((r) => ({
    snapshotId: r.snapshot_id,
    effortId: r.effort.id,
    workItem: r.effort.work_item,
    tasksId: taskIdOfWorkItemRef(r.effort.work_item),
    threadId: r.effort.thread_id,
    startSnapshotId: r.effort.start_snapshot_id,
    endSnapshotId: r.effort.end_snapshot_id,
    completedHere: r.effort.end_snapshot_id === r.snapshot_id,
  }));
}

/** An effort row as the IPC sends it (snake_case; see the generated
 *  `Effort` binding). */
interface RawEffort {
  id: string;
  work_item: string;
  thread_id: string;
  started_at: string;
  ended_at: string | null;
  start_snapshot_id: number | null;
  end_snapshot_id: number | null;
  summary: string | null;
}

function toOverlappingEffort(r: RawEffort): OverlappingEffort {
  return {
    effortId: r.id,
    workItem: r.work_item,
    taskId: taskIdOfWorkItemRef(r.work_item),
    threadId: r.thread_id,
    startedAt: r.started_at,
    endedAt: r.ended_at,
    startSnapshotId: r.start_snapshot_id,
    endSnapshotId: r.end_snapshot_id,
    summary: r.summary,
  };
}

export interface OverlappingEffort {
  effortId: string;
  /** The effort's work item ref. */
  workItem: string;
  /** The oxplow task behind `workItem`; `null` for another provider's item. */
  taskId: string | null;
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
  ) as unknown as RawEffort[];
  return rows.map(toOverlappingEffort);
}

/** Restore a captured file (a `file_snapshot` id) into its stream's
 *  worktree. */
export async function restoreFileSnapshot(fileSnapshotId: number): Promise<void> {
  unwrap(await commands.restoreFileSnapshot(fileSnapshotId));
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

export async function listWorkspaceEntries(streamId: string, path = ""): Promise<WorkspaceEntry[]> {
  return unwrap(
    await commands.listWorkspaceEntries(streamId || null, path),
  );
}

export async function listWorkspaceFiles(streamId: string): Promise<{
  files: WorkspaceIndexedFile[];
  summary: StatusCounts;
}> {
  const [filesRes, summary] = await Promise.all([
    commands.listWorkspaceFiles(streamId || null),
    vcsStatus(streamId),
  ]);
  return { files: unwrap(filesRes), summary: countStatus(summary) };
}

export async function readWorkspaceFile(streamId: string, path: string): Promise<WorkspaceFile> {
  return unwrap(await commands.readWorkspaceFile(streamId || null, path));
}

/** `path` at `revision` (`working`, `snap:<id>`, `git:<rev>`); null
 *  when it isn't there. Every read names its revision — there is no
 *  implicit working-tree default. */
export async function readAt(
  streamId: string,
  path: string,
  revision: Revision,
): Promise<string | null> {
  return unwrap(await commands.readAt(streamId || null, path, revision));
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

export function subscribeGitRefsEvents(
  streamId: string,
  onEvent: () => void,
): () => void {
  return subscribeOxplowEvents((event) => {
    if (event.kind !== "vcsRefsChanged") return;
    if (event.streamId !== streamId) return;
    onEvent();
  });
}

export type AgentStatus = "working" | "waiting" | "stalled" | "awaiting";

export interface AgentStatusEntry {
  streamId: string;
  threadId: string;
  status: AgentStatus;
  /// The `await_user` question text, present only when `status` is
  /// "awaiting". Surfaced as the rail dot's tooltip so you can see what
  /// a (possibly different) thread is asking without switching to it.
  question?: string;
}

/// Collapse the backend `AgentStatusState` enum to the dot's alphabet.
/// "running" → working; "stalled" (derived when a Running hook log
/// goes silent past the stall threshold — the agent died without ever
/// emitting a Stop hook) stays distinct so the dot can render it as a
/// failure rather than ordinary waiting; "awaiting_user" (the agent
/// called `mcp__oxplow__await_user`) stays distinct so the dot can
/// render "waiting on you"; everything else (idle / stopped / error) →
/// waiting.
export function collapseAgentStatusState(raw: string | undefined): AgentStatus {
  if (raw === "running") return "working";
  if (raw === "stalled") return "stalled";
  if (raw === "awaiting_user") return "awaiting";
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
      decision: null,
    }),
  );
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
      streamId: "",
      threadId: row.thread_id,
      // detail is the await_user question only while awaiting; other
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
    // streamId filter is a no-op — the event doesn't carry stream
    // attribution. The single caller in App.tsx subscribes with "all".
    void streamId;
    onEvent({
      streamId: "",
      threadId,
      status,
      question: status === "awaiting" ? (detail ?? undefined) : undefined,
    });
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

export async function probeDaemon(): Promise<boolean> {
  try {
    unwrap(await commands.ping());
    return true;
  } catch {
    return false;
  }
}

/** One logged agent event (`agent.*` in the event log): a prompt, a tool
 *  request or finish, a turn's end, a session, a status change. */
export type AgentEvent = import("./tauri-bridge/generated/bindings.js").StoredEvent;

/** The newest agent activity, newest first — for a stream when given
 *  (the activity log page), else everywhere. */
export async function listAgentEvents(streamId?: string | null, limit = 200): Promise<AgentEvent[]> {
  return unwrap(await commands.listAgentEvents(null, streamId ?? null, limit));
}

/** Load the activity log now and again whenever the backend logs agent
 *  activity (`hookEventsChanged` is a payload-free "something landed"
 *  ping). Only the newest load is delivered, so a slow older one never
 *  overwrites it; a failed load goes to `onError`. */
export function subscribeAgentEvents(
  streamId: string | null,
  limit: number,
  onEvents: (events: AgentEvent[]) => void,
  onError: (err: unknown) => void,
): () => void {
  const load = latestWins(() => listAgentEvents(streamId, limit), onEvents, onError);
  load.run();
  const unsubscribe = subscribeOxplowEvents((event) => {
    if (event.kind === "hookEventsChanged") load.run();
  });
  return () => {
    load.close();
    unsubscribe();
  };
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

// ----------------------------------------------------------------------
// The event log's dead-letter queue (.context/data-model.md "event_log").
// Read through `v_event_dead_letter` (delivery.ts); a person retries or
// discards here.
// ----------------------------------------------------------------------

export type DeadLetter = import("./tauri-bridge/generated/bindings.js").DeadLetter;

/** Run the parked event through its consumer again; returns the letter's new state. */
export async function retryDeadLetter(id: number): Promise<DeadLetter> {
  return unwrap(await commands.retryDeadLetter(id));
}

/** Give up on the parked event (stays visible as `discarded`). */
export async function discardDeadLetter(id: number): Promise<DeadLetter> {
  return unwrap(await commands.discardDeadLetter(id));
}

/** Pages this source points AT. Inverse of `listBacklinks`. */
export async function listPageOutbound(
  sourceKind: string,
  sourceId: string,
  limit: number | null = null,
): Promise<BacklinkEdge[]> {
  return unwrap(await commands.listOutbound(sourceKind, sourceId, limit));
}
