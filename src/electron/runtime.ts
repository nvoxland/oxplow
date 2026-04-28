import { EventEmitter } from "node:events";
import { existsSync, mkdirSync, readFileSync, rmSync, watch, writeFileSync, type FSWatcher } from "node:fs";
import { basename, dirname, join, resolve, sep } from "node:path";
import { randomUUID } from "node:crypto";
import { buildAgentCommandForSession } from "../agent/agent-command.js";
import { buildWriteGuardResponse, NON_WRITER_PROMPT_BLOCK } from "./write-guard.js";
import { buildFilingEnforcementPreToolDeny } from "./filing-enforcement.js";
import { decideStopDirective, type ThreadSnapshot } from "./stop-hook-pipeline.js";
import { ensureAgentPane } from "../terminal/fleet.js";
import { ThreadStore, type Thread, type ThreadState } from "../persistence/thread-store.js";
import {
  detectBaseBranch,
  ensureWorktree,
  isGitRepo,
  isGitWorktree,
  checkoutBranch,
  listBranches,
  listBranchChanges,
  listGitStatuses,
  readFileAtRef,
  getGitLog,
  getCommitDetail,
  getChangeScopes,
  searchWorkspaceText,
  restorePath,
  addPath,
  appendToGitignore,
  gitPush,
  gitPushAsync,
  gitPull,
  gitPullAsync,
  gitFetchAsync,
  gitCommitAll,
  listFileCommits,
  gitBlame,
  listAllRefs,
  listGitRefsGrouped,
  renameBranch,
  deleteBranch,
  gitMergeAsync,
  gitRebaseAsync,
  listExistingWorktrees,
  getAheadBehind,
  getCommitsAheadOf,
  listRecentRemoteBranches,
  gitPushCurrentToAsync,
  gitPullRemoteIntoCurrent,
  type GitWorktreeEntry,
  type RemoteBranchEntry,
  type BranchChanges,
  type GroupedGitRefs,
  type ChangeScopes,
  type CommitDetail,
  type GitLogCommit,
  type GitOpResult,
  type RefOption,
  type TextSearchHit,
} from "../git/git.js";
import { HookEventStore, ingestHookPayload } from "../session/hook-ingest.js";
import { EditorFocusStore, formatEditorFocusForAgent, type EditorFocusState } from "../session/editor-focus.js";
import { deriveThreadAgentStatus, type AgentStatus } from "../session/agent-status.js";
import { LspSessionManager, registerLanguageServer } from "../lsp/lsp.js";
import { createUiClientLogger, createDaemonLogger, type Logger, type LogLevel } from "../core/logger.js";
import { decideResumeUpdate } from "../session/resume-tracker.js";
import { createElectronPlugin, HOOK_EVENTS, type ElectronPlugin } from "../session/claude-plugin.js";
import { startMcpServer, type HookEnvelope, type McpServerHandle } from "../mcp/mcp-server.js";
import { buildWorkItemMcpTools } from "../mcp/mcp-tools.js";
import { buildLspMcpTools } from "../mcp/lsp-mcp-tools.js";
import { getStateDatabase } from "../persistence/state-db.js";
import { StreamStore, type PaneKind, type Stream } from "../persistence/stream-store.js";
import { BACKLOG_SCOPE, WorkItemStore, type WorkItem } from "../persistence/work-item-store.js";
import { WikiNoteStore, computeFreshness as computeWikiNoteFreshness, type WikiNote } from "../persistence/wiki-note-store.js";
import { WikiNoteThreadUpdateStore } from "../persistence/wiki-note-thread-update-store.js";
import { UsageStore } from "../persistence/usage-store.js";
import {
  PageVisitStore,
  type CountByDayOpts,
  type CountByDayRow,
  type ListRecentOpts,
  type PageVisit,
  type PageVisitInput,
  type TopVisitedOpts,
  type TopVisitedRow,
} from "../persistence/page-visit-store.js";
import {
  CodeQualityStore,
  type CodeQualityFindingRow,
  type CodeQualityScanRow,
  type CodeQualityScope,
  type CodeQualityTool,
} from "../persistence/code-quality-store.js";
import {
  CodeQualityToolMissingError,
  runJscpd,
  runLizard,
} from "../subprocess/code-quality.js";
import { NotesWatcher, hashWorkspaceFile, notesDir as wikiNotesDir, syncNoteFromDisk } from "../git/notes-watch.js";
import { buildWikiNoteMcpTools } from "../mcp/wiki-note-mcp-tools.js";
import { WorkItemEffortStore } from "../persistence/work-item-effort-store.js";
import { FollowupStore, type Followup } from "./followup-store.js";
import { BackgroundTaskStore } from "./background-task-store.js";
import {
  SnapshotStore,
  type FileSnapshot,
  type SnapshotDiffResult,
  type SnapshotSource,
  type SnapshotSummary,
} from "../persistence/snapshot-store.js";
import { isInsideWorktree } from "./runtime-paths.js";
import { shouldIgnoreWorkspaceWatchPath } from "../git/workspace-watch.js";
import { createWorkItemApi, type WorkItemApi } from "./work-item-api.js";
import { computeLocalBlame } from "./local-blame.js";
import { EventBus, type OxplowEvent } from "../core/event-bus.js";
import {
  createWorkspaceDirectory,
  createWorkspaceFile,
  deleteWorkspacePath,
  listWorkspaceEntries,
  listWorkspaceFiles,
  readWorkspaceFile,
  renameWorkspacePath,
  summarizeGitStatuses,
  writeWorkspaceFile,
} from "../git/workspace-files.js";
import { WorkspaceWatcherRegistry } from "../git/workspace-watch.js";
import { GitRefsWatcherRegistry } from "../git/git-refs-watch.js";
import { detectCurrentBranch, readWorktreeHeadSha } from "../git/git.js";
import { loadProjectConfig, writeProjectConfig, type OxplowConfig } from "../config/config.js";
import { killSession } from "../terminal/tmux.js";
import { attachPane } from "../terminal/pty-bridge.js";
import { AgentPtyStore } from "../terminal/agent-pty-store.js";
import type { EditorFocusPayload, UiLogPayload } from "./ipc-contract.js";

export class ElectronRuntime {
  readonly projectDir: string;
  readonly projectBase: string;
  readonly logger: Logger;
  readonly store: StreamStore;
  readonly threadStore: ThreadStore;
  private readonly workItemStore: WorkItemStore;
  readonly wikiNoteStore: WikiNoteStore;
  readonly wikiNoteThreadUpdateStore: WikiNoteThreadUpdateStore;
  readonly usageStore: UsageStore;
  readonly pageVisitStore: PageVisitStore;
  readonly codeQualityStore: CodeQualityStore;
  private notesWatcher: NotesWatcher | null = null;
  readonly effortStore: WorkItemEffortStore;
  readonly snapshotStore: SnapshotStore;
  /** Transient in-memory follow-up store. Backs the agent-only
   *  `add_followup` / `remove_followup` MCP tools and surfaces entries
   *  in the To Do section of the Work panel. No DB row, lost on
   *  runtime restart — see followup-store.ts. */
  readonly followupStore: FollowupStore;
  /** Transient in-memory progress rows for long-running ops (git
   *  pull/push/fetch, code-quality scans, LSP startup, notes resync).
   *  Surfaced in the bottom bar; lost on restart. See
   *  background-task-store.ts. */
  readonly backgroundTaskStore: BackgroundTaskStore;
  readonly workItemApi: WorkItemApi;
  readonly hookEvents: HookEventStore;
  readonly lspManager: LspSessionManager;
  readonly editorFocusStore: EditorFocusStore;
  readonly agentPtyStore: AgentPtyStore;
  readonly workspaceWatchers: WorkspaceWatcherRegistry;
  readonly gitRefsWatchers: GitRefsWatcherRegistry;
  config: OxplowConfig;
  readonly events: EventBus;

  private electronPlugin: ElectronPlugin | null = null;
  private readonly terminalSessions = new Map<string, RuntimeSocket>();
  /** Tracks which thread each terminal-pane websocket is attached to so
   *  Escape / Ctrl-C keystrokes can be synthesized into an Interrupt
   *  meta hook event for the right thread. Populated in
   *  `openTerminalSession`, removed on close. */
  private readonly terminalSessionThread = new Map<string, { streamId: string; threadId: string }>();
  private readonly lspClients = new Map<string, RuntimeSocket>();
  private readonly agentStatusByThread = new Map<string, AgentStatus>();
  private readonly recentUiWrites = new Map<string, number>();
  private readonly dirtyPathsByStream = new Map<string, Set<string>>();
  /** Last <session-context> block we sent to each Claude session. Used to
   *  skip re-injecting identical blocks across turns. Keyed by Claude
   *  session id; absent key means "nothing sent yet / fall back to emit". */
  private readonly lastSessionContextBySessionId = new Map<string, string>();
  /** The thread's writer/read-only role at the moment a Claude session id was
   *  first seen. Captured once and never rewritten, so
   *  buildSessionContextBlock can detect a mid-session role flip and emit a
   *  loud banner superseding the frozen NON_WRITER block in the agent's
   *  initial system prompt. Keyed by Claude session id. */
  private readonly initialRoleBySessionId = new Map<string, "writer" | "read-only">();
  /** Per-thread activity flag for the current turn. Seeded `false` on
   *  UserPromptSubmit, flipped to `true` on the first qualifying
   *  PostToolUse (write-intent / oxplow filing / dispatch — see
   *  `isActivityTool`). Read on Stop: `false` means the turn was pure
   *  Q&A (the agent answered or asked a question) and the entire
   *  Stop-directive pipeline is suppressed so the agent stays stopped
   *  waiting for the user. Absent key (no UserPromptSubmit yet) is
   *  treated as "unknown → don't suppress" so behaviour stays stable
   *  in tests and edge cases. */
  private readonly turnActivityByThread = new Map<string, boolean>();
  /** Per-thread count of `Task` (subagent) tool calls that are
   *  PreToolUse-started but haven't yet seen a PostToolUse. Used by the
   *  Stop-hook pipeline to suppress the in-progress audit while a
   *  subagent owns the in_progress item — re-firing the nudge produces
   *  a visual loop where the parent acks each Stop while still waiting
   *  on the subagent. See wi-593a50b62e22. */
  private readonly pendingSubagentsByThread = new Map<string, number>();
  /** Per-thread fingerprint of the in_progress set the runtime last
   *  emitted an audit directive for. The Stop pipeline compares the
   *  current snapshot against this and suppresses a duplicate audit
   *  nudge when nothing changed (no item's `updated_at` ticked, no note
   *  landed, set membership is identical). Cleared lazily by the next
   *  audit fire that records a fresh signature. See wi-c468e8fc093d. */
  private readonly lastAuditSignatureByThread = new Map<string, string>();
  /** Per-thread write counter for the current turn. Bumped on PostToolUse
   *  for any write-intent tool (Edit/Write/MultiEdit/non-readonly Bash).
   *  Drives `turnHadWrites` for the filing-enforcement Stop branch.
   *  Cleared on UserPromptSubmit alongside `turnActivityByThread`. */
  private readonly turnWriteCountByThread = new Map<string, number>();
  /** Per-thread "agent is awaiting the user" flag. Set by the
   *  `await_user` MCP tool; consumed by Stop hook (allows-stop +
   *  suppresses every directive); cleared on UserPromptSubmit. The
   *  question text is stored alongside for telemetry / future UI
   *  surfacing — the Stop hook only reads the boolean. */
  private readonly awaitingUserByThread = new Map<string, { question: string; setAt: number }>();
  /** Per-thread "filed/transitioned a work item this turn" flag. Set
   *  when the agent calls any work-item-mutating MCP tool
   *  (create_work_item, update_work_item, complete_task,
   *  file_epic_with_children, transition_work_items, dispatch_work_item).
   *  Consumed by Stop hook's filing-enforcement branch. Cleared on
   *  UserPromptSubmit alongside the other per-turn flags. */
  private readonly filedThisTurnByThread = new Set<string>();
  /** Per-thread "filed at least one new ready item this turn (no
   *  in_progress claim attached)" flag. Stricter subset of
   *  `filedThisTurnByThread`: set only by `create_work_item` /
   *  `file_epic_with_children` when the new row landed at `ready`
   *  (the default). Drives the Stop-hook "filed but didn't ship"
   *  advisory branch — catches turns where the agent logged work as
   *  backlog when the user's instruction was to do it. Cleared
   *  alongside the other per-turn flags on UserPromptSubmit. */
  private readonly filedReadyThisTurnByThread = new Set<string>();
  /** Per-thread "the filed-but-didn't-ship Stop advisory has already
   *  fired in this prompt gap" flag. Set by a pipeline side effect
   *  after the advisory fires; checked on the next Stop to suppress
   *  the loop where the agent acks the advisory and the Stop hook
   *  immediately re-blocks with the same message. Cleared on
   *  UserPromptSubmit alongside the other per-turn filing flags. */
  private readonly filedButDidntShipFiredByThread = new Set<string>();
  private mcp: McpServerHandle | null = null;
  private gitEnabledCached = false;
  private gitRootWatcher: FSWatcher | null = null;
  private snapshotCleanupTimer: ReturnType<typeof setInterval> | null = null;
  /** Dev-time watchers for src/mcp and src/persistence; created only when
   *  `OXPLOW_DEV_RELOAD=1` AND the runtime can resolve a source tree
   *  (checked via `findSourceRoot`). Close in dispose(). */
  private devReloadWatchers: FSWatcher[] = [];
  /** Coalesces bursty fs events (save-on-build, git checkout) into a
   *  single restart attempt. */
  private devReloadTimer: ReturnType<typeof setTimeout> | null = null;
  /** Guards against overlapping restarts — fs events keep firing while
   *  the restart is in-flight. */
  private devReloadInFlight = false;
  /** ISO timestamp captured in `initialize()`. Used as the cutoff for
   *  `turnStore.listOpenTurns` so orphaned open turns from prior runs
   *  (oxplow crashed mid-turn, `ended_at` never set) don't haunt the UI's
   *  in_progress bucket. See the passive-turn-tracking plan. */
  startedAt: string = "";
  private disposed = false;

  private constructor(projectDir: string, projectBase: string, logger: Logger, config: OxplowConfig) {
    this.projectDir = projectDir;
    this.projectBase = projectBase;
    this.logger = logger;
    this.config = config;
    this.store = new StreamStore(projectDir, logger.child({ subsystem: "stream-store" }));
    this.threadStore = new ThreadStore(projectDir, logger.child({ subsystem: "thread-store" }));
    this.workItemStore = new WorkItemStore(projectDir, logger.child({ subsystem: "work-items" }));
    this.wikiNoteStore = new WikiNoteStore(projectDir, logger.child({ subsystem: "wiki-notes" }));
    this.wikiNoteThreadUpdateStore = new WikiNoteThreadUpdateStore(projectDir, logger.child({ subsystem: "wiki-note-thread-updates" }));
    this.usageStore = new UsageStore(projectDir, logger.child({ subsystem: "usage" }));
    this.pageVisitStore = new PageVisitStore(projectDir, logger.child({ subsystem: "page-visit" }));
    this.codeQualityStore = new CodeQualityStore(projectDir, logger.child({ subsystem: "code-quality" }));
    this.backgroundTaskStore = new BackgroundTaskStore(logger.child({ subsystem: "background-task-store" }));
    let notesScanTaskId: string | null = null;
    this.notesWatcher = new NotesWatcher(
      projectDir,
      this.wikiNoteStore,
      {
        onScanStart: (total) => {
          // Skip the row entirely for tiny note dirs — no UI value, just churn.
          if (total < 5) return;
          notesScanTaskId = this.backgroundTaskStore.start({
            kind: "notes-resync",
            label: "Syncing wiki notes…",
            detail: `0 / ${total}`,
            progress: 0,
          });
        },
        onScanProgress: ({ done, total, slug }) => {
          if (!notesScanTaskId) return;
          this.backgroundTaskStore.update(notesScanTaskId, {
            progress: total > 0 ? done / total : null,
            detail: `${done} / ${total} — ${slug}`,
          });
        },
        onScanEnd: (error) => {
          if (!notesScanTaskId) return;
          if (error) this.backgroundTaskStore.fail(notesScanTaskId, error.message);
          else this.backgroundTaskStore.complete(notesScanTaskId);
          notesScanTaskId = null;
        },
      },
      logger.child({ subsystem: "notes-watch" }),
    );
    this.notesWatcher.start();
    this.effortStore = new WorkItemEffortStore(projectDir, logger.child({ subsystem: "effort-store" }));
    this.snapshotStore = new SnapshotStore(projectDir, logger.child({ subsystem: "snapshot-store" }));
    this.snapshotStore.setMaxFileBytes(config.snapshotMaxFileBytes);
    this.followupStore = new FollowupStore();
    this.workItemApi = createWorkItemApi({
      resolveThread: (streamId, threadId) => this.resolveThread(streamId, threadId),
      workItemStore: this.workItemStore,
      effortStore: this.effortStore,
      snapshotStore: this.snapshotStore,
      followupStore: this.followupStore,
    });
    this.events = new EventBus(logger.child({ subsystem: "event-bus" }));
    this.hookEvents = new HookEventStore(1000);
    this.lspManager = new LspSessionManager(logger.child({ subsystem: "lsp" }), {
      onInitializeStart: (languageId) => this.backgroundTaskStore.start({
        kind: "lsp",
        label: `Starting ${languageId} language server…`,
      }),
      onInitializeEnd: (taskHandle, error) => {
        if (!taskHandle) return;
        if (error) this.backgroundTaskStore.fail(taskHandle, error.message);
        else this.backgroundTaskStore.complete(taskHandle);
      },
    });
    this.editorFocusStore = new EditorFocusStore();
    this.agentPtyStore = new AgentPtyStore();
    this.workspaceWatchers = new WorkspaceWatcherRegistry(logger.child({ subsystem: "workspace-watch" }));
    this.gitRefsWatchers = new GitRefsWatcherRegistry(logger.child({ subsystem: "git-refs-watch" }));
  }

  // Hook delivery health — used to warn once per process when the registered
  // set in HOOK_EVENTS doesn't match what Claude Code actually ships. The
  // canonical example is SessionStart, which Claude silently drops for HTTP
  // hooks (see .context/agent-model.md); anyone hitting a similar dead
  // registration learns about it from the log instead of by running
  // `claude --debug-file` and chasing payload shapes.
  private readonly hookEventsSeen = new Set<string>();
  private hookHealthReported = false;

  static async create(projectDir: string): Promise<ElectronRuntime> {
    // oxplow manages its own worktrees under .oxplow/worktrees/; refusing to
    // boot inside someone else's worktree keeps the stream/pane accounting
    // from getting tangled with a foreign git checkout.
    if (isGitWorktree(projectDir)) {
      throw new Error(
        `oxplow cannot run inside a git worktree (${projectDir}). Open it from the main repository checkout or from a directory that isn't under git.`,
      );
    }
    const logger = createDaemonLogger(projectDir).child({ pid: process.pid, subsystem: "electron-runtime" });
    const config = loadProjectConfig(projectDir, logger.child({ subsystem: "config" }));
    for (const server of config.lspServers) {
      registerLanguageServer(server);
      logger.info("registered lsp server from config", {
        languageId: server.languageId,
        extensions: server.extensions,
      });
    }
    const projectBase = sanitizeProjectBase(projectDir);
    const runtime = new ElectronRuntime(projectDir, projectBase, logger, config);
    await runtime.initialize();
    return runtime;
  }

  private async initialize(): Promise<void> {
    this.startedAt = new Date().toISOString();
    const gitWorkspace = isGitRepo(this.projectDir);
    const branch = gitWorkspace ? detectCurrentBranch(this.projectDir) ?? this.projectBase : this.projectBase;
    const branchRef = gitWorkspace ? `refs/heads/${branch}` : branch;

    // Primary stream is the repo itself: worktree_path === projectDir,
    // title === repo basename, kind === "primary". Its recorded branch
    // tracks whatever HEAD is currently pointing at and is updated live
    // by the git-refs watcher when HEAD moves.
    let stream = this.store.findPrimary();
    if (!stream) {
      stream = this.store.create({
        title: this.projectBase,
        branch,
        branchRef,
        branchSource: "local",
        worktreePath: this.projectDir,
        projectBase: this.projectBase,
        kind: "primary",
      });
      this.logger.info("created primary stream", { streamId: stream.id, branch });
    } else if (stream.branch !== branch) {
      stream = this.store.setStreamBranch(stream.id, branch, branchRef);
      this.logger.info("primary stream branch re-synced on boot", { streamId: stream.id, branch });
    } else {
      this.logger.info("reusing primary stream", { streamId: stream.id, branch });
    }

    this.store.ensureCurrentStreamId(stream.id);
    cleanupSessions(this.store.list());
    this.logger.info("initialized current stream", { streamId: this.store.getCurrentStreamId() });

    // Push the user's `generatedDirs` down to the watcher before starting
    // any stream-scoped watchers so they pick up the filter from the first
    // event.
    this.workspaceWatchers.setExtraIgnoreDirs(this.config.generatedDirs);

    for (const existingStream of this.store.list()) {
      this.threadStore.ensureStream(existingStream);
      this.workspaceWatchers.ensureWatching(existingStream);
      this.gitRefsWatchers.ensureWatching(existingStream);
      this.takeStartupSnapshot(existingStream.id);
    }

    this.workspaceWatchers.subscribe((event) => {
      this.events.publish({
        type: "workspace.changed",
        id: event.id,
        streamId: event.streamId,
        kind: event.kind,
        path: event.path,
        t: event.t,
      });
      // Track into the snapshot dirty set regardless of active thread state —
      // edits between turns still need to show up in the next turn-start
      // snapshot so the agent's "before" is accurate.
      this.markDirty(event.streamId, event.path);
    });
    this.gitRefsWatchers.subscribe((change) => {
      this.events.publish({
        type: "git-refs.changed",
        streamId: change.streamId,
        t: change.t,
      });
      this.maybeSyncStreamBranch(change.streamId);
    });
    this.hookEvents.subscribe((event) => {
      this.events.publish({
        type: "hook.recorded",
        streamId: event.streamId,
        threadId: event.threadId,
        pane: event.pane,
        event,
      });
      if (event.threadId) this.recomputeAgentStatus(event.streamId, event.threadId);
    });
    this.workItemStore.subscribe((change) => {
      if (change.threadId === BACKLOG_SCOPE) {
        this.events.publish({
          type: "backlog.changed",
          kind: change.kind,
          itemId: change.itemId,
        });
        return;
      }
      const thread = this.threadStore.findById(change.threadId);
      if (!thread) return;
      if (change.kind === "updated" && change.itemId && change.previousStatus !== change.nextStatus) {
        this.handleStatusTransition(
          thread.stream_id,
          change.threadId,
          change.itemId,
          change.previousStatus,
          change.nextStatus,
          change.touchedFiles,
        );
      }
      if (change.kind === "created") {
        // Capture a task-event snapshot on creation so Local History
        // shows the moment the row appeared. Gap-gated by the same 5-min
        // rule as task-end (see applyStatusTransition) — back-to-back
        // creates / status changes don't pile up near-identical rows.
        const lastSnapTs = this.snapshotStore.getMostRecentSnapshotTimestampForStream(thread.stream_id);
        const skip = lastSnapTs !== null && shouldSkipEndSnapshot(lastSnapTs, Date.now());
        if (!skip) this.safeFlushSnapshot(thread.stream_id, "task-event", null);
      }
      this.events.publish({
        type: "work-item.changed",
        streamId: thread.stream_id,
        threadId: change.threadId,
        kind: change.kind,
        itemId: change.itemId,
      });
    });
    this.threadStore.subscribe((change) => {
      this.events.publish({
        type: "thread.changed",
        streamId: change.streamId,
        threadId: change.threadId,
        kind: change.kind,
      });
    });
    this.store.subscribe((change) => {
      this.events.publish({ type: "stream.changed", kind: change.kind, streamId: change.streamId });
    });
    this.followupStore.subscribe((change) => {
      this.events.publish({
        type: "followup.changed",
        threadId: change.threadId,
        kind: change.kind,
        id: change.id,
      });
    });
    this.backgroundTaskStore.subscribe((change) => {
      this.events.publish({
        type: "background-task.changed",
        kind: change.kind,
        id: change.id,
      });
    });
    this.wikiNoteStore.subscribe((change) => {
      this.events.publish({
        type: "wiki-note.changed",
        kind: change.kind,
        slug: change.slug,
      });
    });
    this.usageStore.subscribe((change) => {
      this.events.publish({
        type: "usage.recorded",
        kind: change.kind,
        key: change.key,
        streamId: change.streamId,
        threadId: change.threadId,
      });
    });
    this.pageVisitStore.subscribe((change) => {
      this.events.publish({
        type: "page-visit.changed",
        refId: change.refId,
        refKind: change.refKind,
        threadId: change.threadId,
      });
    });
    this.codeQualityStore.subscribe((change) => {
      this.events.publish({
        type: "code-quality.scanned",
        streamId: change.streamId,
        scanId: change.scanId,
        tool: change.tool,
        scope: change.scope,
        status: change.kind === "started" ? "running" : change.kind,
      });
    });

    this.gitEnabledCached = isGitRepo(this.projectDir);
    // Watch the project root for the `.git` direntry appearing or
    // disappearing so the UI reacts when the agent `git init`s the project
    // mid-session. We can't lean on `workspaceWatchers` because it filters
    // `.git` out by design; this is a tiny direct `fs.watch` scoped to the
    // root dir, and we only re-check when the changed filename is `.git`.
    try {
      this.gitRootWatcher = watch(this.projectDir, (_event, filename) => {
        const name = typeof filename === "string"
          ? filename
          : filename != null
            ? (filename as Buffer).toString("utf8")
            : "";
        if (name !== ".git") return;
        const next = isGitRepo(this.projectDir);
        if (next === this.gitEnabledCached) return;
        this.gitEnabledCached = next;
        this.events.publish({ type: "workspace-context.changed", gitEnabled: next });
        // A `.git` directory just appeared (or disappeared). Re-bind refs
        // watchers for every stream rooted here so commits made right after
        // `git init` start auto-refreshing the UI.
        for (const s of this.store.list()) {
          if (next) this.gitRefsWatchers.ensureWatching(s);
          else this.gitRefsWatchers.stopWatching(s.id);
        }
      });
      this.gitRootWatcher.on("error", (error) => {
        this.logger.warn("git root watcher error", {
          error: error instanceof Error ? error.message : String(error),
        });
      });
    } catch (error) {
      this.logger.warn("failed to start git root watcher", {
        error: error instanceof Error ? error.message : String(error),
      });
    }

    // Snapshot retention: prune on startup, then once per 24h while running.
    // `snapshotRetentionDays === 0` disables pruning.
    this.runSnapshotCleanup();
    this.snapshotCleanupTimer = setInterval(() => this.runSnapshotCleanup(), 24 * 60 * 60 * 1000);

    this.mcp = await startMcpServer({
      workspaceFolders: this.store.list().map((candidate) => candidate.worktree_path),
      logger: this.logger.child({ subsystem: "mcp" }),
      extraTools: [
        ...buildWorkItemMcpTools({
          resolveStream: (streamId) => this.resolveStream(streamId),
          resolveThreadById: (threadId) => this.resolveThreadById(threadId),
          threadStore: this.threadStore,
          streamStore: this.store,
          workItemStore: this.workItemStore,
          effortStore: this.effortStore,
          markAwaitingUser: (threadId, question) => this.markAwaitingUser(threadId, question),
          markFiledThisTurn: (threadId) => this.markFiledThisTurn(threadId),
          markFiledReadyThisTurn: (threadId) => this.markFiledReadyThisTurn(threadId),
          forkThread: (input) => this.forkThread(input),
          followupStore: this.followupStore,
        }),
        ...buildLspMcpTools({
          resolveStream: (streamId) => this.resolveStream(streamId),
          lspManager: this.lspManager,
        }),
        ...buildWikiNoteMcpTools({
          resolveStream: (streamId) => this.resolveStream(streamId),
          wikiNoteStore: this.wikiNoteStore,
          recordNoteUpdate: (slug, threadId) => this.wikiNoteThreadUpdateStore.recordUpdate(slug, threadId),
        }),
      ],
      onHook: (envelope) => this.handleHookEnvelope(envelope),
    });
    this.logger.info("started mcp server", { port: this.mcp.port, lockfilePath: this.mcp.lockfilePath });

    this.maybeStartDevReloadWatchers();
  }

  /**
   * Dev-only: when `OXPLOW_DEV_RELOAD=1` and a source tree is resolvable
   * from `process.cwd()`, watch `src/mcp/` and `src/persistence/` and
   * restart the MCP server on change. The restart re-runs
   * `buildWorkItemMcpTools` / `buildLspMcpTools` and rebinds the TCP
   * port + lockfile so Claude Code's next `tools/list` sees whatever
   * was rebuilt.
   *
   * **Known limitation (tracked on this task):** ESM module caching
   * means the re-imports inside `buildWorkItemMcpTools` return the
   * *same* in-memory module graph as before the change — so a dev
   * editing tool logic still needs a full process restart to actually
   * pick up new handler code. What this does buy you: rebinding the
   * port after the lockfile goes stale, and a loud "source touched"
   * log so the dev knows a full restart is due. Full hot-reload
   * requires either a child-process MCP model or bun --hot style
   * process reload; both are bigger changes than this QoL hook
   * warrants. See wi-4c3a6289871f for context.
   *
   * Zero runtime cost when the env var is unset — we don't even
   * resolve the source root in that case.
   */
  private maybeStartDevReloadWatchers(): void {
    if (process.env.OXPLOW_DEV_RELOAD !== "1") return;
    const sourceRoot = findSourceRoot();
    if (!sourceRoot) {
      this.logger.warn("OXPLOW_DEV_RELOAD=1 but no source tree found; dev-reload disabled");
      return;
    }
    const watched = [join(sourceRoot, "src", "mcp"), join(sourceRoot, "src", "persistence")];
    for (const dir of watched) {
      try {
        const w = watch(dir, { recursive: true }, (_eventType, filename) => {
          if (!filename) return;
          // Ignore vim swap/save-through noise. Only .ts/.tsx changes
          // could plausibly affect handler behaviour.
          const name = String(filename);
          if (!(name.endsWith(".ts") || name.endsWith(".tsx"))) return;
          this.scheduleDevReload(name);
        });
        w.on("error", (error) => {
          this.logger.warn("dev-reload watcher error", {
            dir,
            error: error instanceof Error ? error.message : String(error),
          });
        });
        this.devReloadWatchers.push(w);
        this.logger.info("dev-reload watcher started", { dir });
      } catch (error) {
        this.logger.warn("failed to start dev-reload watcher", {
          dir,
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }
  }

  /** Debounced restart scheduler — coalesces bursty save events. */
  private scheduleDevReload(trigger: string): void {
    if (this.devReloadTimer) clearTimeout(this.devReloadTimer);
    this.devReloadTimer = setTimeout(() => {
      this.devReloadTimer = null;
      void this.runDevReload(trigger);
    }, 250);
  }

  private async runDevReload(trigger: string): Promise<void> {
    if (this.disposed) return;
    if (this.devReloadInFlight) return;
    this.devReloadInFlight = true;
    try {
      this.logger.warn("dev-reload triggered; restarting MCP server", { trigger });
      if (this.mcp) {
        await this.mcp.stop();
        this.mcp = null;
      }
      this.mcp = await startMcpServer({
        workspaceFolders: this.store.list().map((candidate) => candidate.worktree_path),
        logger: this.logger.child({ subsystem: "mcp" }),
        extraTools: [
          ...buildWorkItemMcpTools({
            resolveStream: (streamId) => this.resolveStream(streamId),
            resolveThreadById: (threadId) => this.resolveThreadById(threadId),
            threadStore: this.threadStore,
            streamStore: this.store,
            workItemStore: this.workItemStore,
            effortStore: this.effortStore,
            markAwaitingUser: (threadId, question) => this.markAwaitingUser(threadId, question),
            markFiledThisTurn: (threadId) => this.markFiledThisTurn(threadId),
            markFiledReadyThisTurn: (threadId) => this.markFiledReadyThisTurn(threadId),
            forkThread: (input) => this.forkThread(input),
          }),
          ...buildLspMcpTools({
            resolveStream: (streamId) => this.resolveStream(streamId),
            lspManager: this.lspManager,
          }),
          ...buildWikiNoteMcpTools({
            resolveStream: (streamId) => this.resolveStream(streamId),
            wikiNoteStore: this.wikiNoteStore,
          }),
        ],
        onHook: (envelope) => this.handleHookEnvelope(envelope),
      });
      this.logger.info("dev-reload complete", { port: this.mcp.port, lockfilePath: this.mcp.lockfilePath });
    } catch (error) {
      this.logger.warn("dev-reload failed", {
        error: error instanceof Error ? error.message : String(error),
      });
    } finally {
      this.devReloadInFlight = false;
    }
  }

  async dispose(): Promise<void> {
    this.disposed = true;
    this.gitRootWatcher?.close();
    this.gitRootWatcher = null;
    for (const w of this.devReloadWatchers) {
      try { w.close(); } catch { /* ignore */ }
    }
    this.devReloadWatchers = [];
    if (this.devReloadTimer) {
      clearTimeout(this.devReloadTimer);
      this.devReloadTimer = null;
    }
    if (this.snapshotCleanupTimer) {
      clearInterval(this.snapshotCleanupTimer);
      this.snapshotCleanupTimer = null;
    }
    cleanupSessions(this.store.list());
    for (const socket of this.terminalSessions.values()) socket.close();
    this.terminalSessions.clear();
    this.agentPtyStore.disposeAll();
    for (const socket of this.lspClients.values()) socket.close();
    this.lspClients.clear();
    this.workspaceWatchers.dispose();
    this.gitRefsWatchers.dispose();
    this.notesWatcher?.dispose();
    this.notesWatcher = null;
    await this.lspManager.dispose();
    if (this.mcp) {
      await this.mcp.stop();
      this.mcp = null;
    }
    getStateDatabase(this.projectDir).close();
  }

  onEvent(listener: (event: OxplowEvent) => void): () => void {
    return this.events.subscribe(listener);
  }

  getThreadAgentStatus(threadId: string): AgentStatus {
    return this.agentStatusByThread.get(threadId) ?? "waiting";
  }

  /**
   * Recently finished work merged across closed work-item efforts
   * (per-thread) and updated wiki notes (global). Drives the rail's
   * "Finished" section. Returns up to `limit` entries sorted by
   * timestamp DESC.
   */
  listRecentlyFinished(threadId: string | null, limit: number):
    Array<
      | { kind: "work-item"; itemId: string; title: string; t: string }
      | { kind: "note"; slug: string; title: string; t: string }
    >
  {
    const db = getStateDatabase(this.projectDir);
    const threadScope = threadId ? `thread:${threadId}` : "thread:all";
    const itemSince = db.get<{ t: string }>(
      `SELECT t FROM finished_seen WHERE scope = ?`,
      threadScope,
    )?.t ?? null;
    const noteSince = db.get<{ t: string }>(
      `SELECT t FROM finished_seen WHERE scope = ?`,
      "notes",
    )?.t ?? null;
    const items = this.effortStore.listRecentClosed(threadId, limit, itemSince).map((row) => ({
      kind: "work-item" as const,
      itemId: row.itemId,
      title: row.title,
      t: row.endedAt,
    }));
    // Notes are global, but the rail attributes each edit to the thread
    // that authored it (mirrors how task efforts attribute via
    // work_item_effort.thread_id). Without a thread we have nothing to
    // attribute against, so the note feed is empty in that case.
    const notes: Array<{ kind: "note"; slug: string; title: string; t: string }> = [];
    if (threadId) {
      for (const row of this.wikiNoteThreadUpdateStore.listRecentByThread(threadId, limit)) {
        const note = this.wikiNoteStore.getBySlug(row.slug);
        if (!note) continue;
        notes.push({
          kind: "note",
          slug: note.slug,
          title: note.title,
          t: row.updated_at,
        });
      }
    }
    return [...items, ...notes]
      .sort((a, b) => (a.t < b.t ? 1 : a.t > b.t ? -1 : 0))
      .slice(0, limit);
  }

  /**
   * Mark all currently-finished entries as seen. Records a watermark
   * (current ISO timestamp) under per-thread + global scopes; subsequent
   * listRecentlyFinished filters out anything ≤ watermark.
   */
  clearRecentlyFinished(threadId: string | null): void {
    const db = getStateDatabase(this.projectDir);
    const now = new Date().toISOString();
    const threadScope = threadId ? `thread:${threadId}` : "thread:all";
    db.run(
      `INSERT INTO finished_seen (scope, t) VALUES (?, ?)
       ON CONFLICT(scope) DO UPDATE SET t = excluded.t`,
      threadScope,
      now,
    );
    db.run(
      `INSERT INTO finished_seen (scope, t) VALUES (?, ?)
       ON CONFLICT(scope) DO UPDATE SET t = excluded.t`,
      "notes",
      now,
    );
    this.events.publish({ type: "work-item.changed", id: null, kind: "list" } as any);
    this.events.publish({ type: "wiki-note.changed", kind: "upserted", slug: "" } as any);
  }

  recordPageVisit(input: PageVisitInput): void {
    this.pageVisitStore.record(input);
  }

  listRecentPageVisits(opts: ListRecentOpts): PageVisit[] {
    return this.pageVisitStore.listRecent(opts);
  }

  topVisitedPages(opts: TopVisitedOpts): TopVisitedRow[] {
    return this.pageVisitStore.topVisited(opts);
  }

  countPageVisitsByDay(opts: CountByDayOpts): CountByDayRow[] {
    return this.pageVisitStore.countByDay(opts);
  }

  listAgentStatuses(streamId?: string): Array<{ streamId: string; threadId: string; status: AgentStatus }> {
    const out: Array<{ streamId: string; threadId: string; status: AgentStatus }> = [];
    for (const [threadId, status] of this.agentStatusByThread) {
      const thread = this.threadStore.findById(threadId);
      if (!thread) continue;
      if (streamId && thread.stream_id !== streamId) continue;
      out.push({ streamId: thread.stream_id, threadId, status });
    }
    return out;
  }

  private recomputeAgentStatus(streamId: string, threadId: string): void {
    const events = this.hookEvents.list(streamId).filter((candidate) => candidate.threadId === threadId);
    const next = deriveThreadAgentStatus(events);
    const prev = this.agentStatusByThread.get(threadId);
    if (prev === next) return;
    this.agentStatusByThread.set(threadId, next);
    this.events.publish({
      type: "agent-status.changed",
      streamId,
      threadId,
      status: next,
    });
  }

  listStreams(): Stream[] {
    return this.store.list();
  }

  getCurrentStream(): Stream {
    const current = this.store.getCurrent();
    if (!current) throw new Error("no current stream");
    return current;
  }

  switchStream(id: string): Stream {
    this.store.setCurrentStreamId(id);
    this.logger.info("switched current stream", { streamId: id });
    const stream = this.store.get(id);
    if (!stream) throw new Error(`unknown stream: ${id}`);
    return stream;
  }

  renameCurrentStream(title: string): Stream {
    const current = this.getCurrentStream();
    return this.renameStream(current.id, title);
  }

  getConfig(): OxplowConfig {
    return this.config;
  }

  setAgentPromptAppend(text: string): OxplowConfig {
    const next: OxplowConfig = { ...this.config, agentPromptAppend: text };
    writeProjectConfig(this.projectDir, next);
    this.config = next;
    this.logger.info("updated agent prompt append", { length: text.length });
    this.events.publish({ type: "config.changed" });
    return next;
  }

  setSnapshotRetentionDays(days: number): OxplowConfig {
    if (!Number.isFinite(days) || days < 0) {
      throw new Error("snapshotRetentionDays must be a non-negative number");
    }
    const next: OxplowConfig = { ...this.config, snapshotRetentionDays: days };
    writeProjectConfig(this.projectDir, next);
    this.config = next;
    this.logger.info("updated snapshot retention days", { days });
    this.events.publish({ type: "config.changed" });
    return next;
  }

  setSnapshotMaxFileBytes(bytes: number): OxplowConfig {
    if (!Number.isFinite(bytes) || bytes < 1024) {
      throw new Error("snapshotMaxFileBytes must be a number >= 1024");
    }
    const next: OxplowConfig = { ...this.config, snapshotMaxFileBytes: Math.floor(bytes) };
    writeProjectConfig(this.projectDir, next);
    this.config = next;
    this.logger.info("updated snapshot max file bytes", { bytes: next.snapshotMaxFileBytes });
    this.events.publish({ type: "config.changed" });
    return next;
  }

  setGeneratedDirs(dirs: string[]): OxplowConfig {
    // Normalize: strip leading/trailing slashes, dedupe, drop empties. Path
    // separators are illegal — single path segments only, per config schema.
    const normalized = Array.from(
      new Set(
        dirs
          .map((entry) => entry.trim().replace(/^\/+|\/+$/g, ""))
          .filter((entry) => entry.length > 0 && !entry.includes("/")),
      ),
    ).sort();
    const next: OxplowConfig = { ...this.config, generatedDirs: normalized };
    writeProjectConfig(this.projectDir, next);
    this.config = next;
    this.workspaceWatchers.setExtraIgnoreDirs(normalized);
    this.logger.info("updated generated dirs", { count: normalized.length });
    this.events.publish({ type: "config.changed" });
    return next;
  }

  renameStream(streamId: string, title: string): Stream {
    const updated = this.store.update(streamId, (stream) => ({ ...stream, title }));
    this.logger.info("renamed stream", { streamId: updated.id, title: updated.title });
    return updated;
  }

  listBranches() {
    return listBranches(this.projectDir);
  }

  getDefaultBranch(): string | null {
    return detectBaseBranch(this.projectDir);
  }

  listGitRefs(): GroupedGitRefs {
    return listGitRefsGrouped(this.projectDir);
  }

  renameGitBranch(from: string, to: string): GitOpResult {
    return renameBranch(this.projectDir, from, to);
  }

  deleteGitBranch(branch: string, options?: { force?: boolean }): GitOpResult {
    return deleteBranch(this.projectDir, branch, options?.force);
  }

  /**
   * Long-running git ops are kickoff-style: the IPC promise resolves
   * immediately with a `taskId` once the BackgroundTaskStore row is
   * registered, and the actual work runs in the background. The renderer
   * subscribes by taskId to drive in-flight UI and reads `task.result`
   * (a `GitOpResult`) once the task ends. This keeps the renderer
   * unblocked while git churns and gives any surface — not just the
   * caller — an authoritative pending signal.
   */
  gitMergeInto(streamId: string, other: string): Promise<{ taskId: string }> {
    const stream = this.resolveStream(streamId);
    const target = stream.branch ?? "HEAD";
    const taskId = this.backgroundTaskStore.start({
      kind: "git",
      label: `Merging ${other} → ${target}…`,
    });
    void this.runGitTask(taskId, "git merge failed", () => gitMergeAsync(stream.worktree_path, other));
    return Promise.resolve({ taskId });
  }

  gitRebaseOnto(streamId: string, onto: string): Promise<{ taskId: string }> {
    const stream = this.resolveStream(streamId);
    const taskId = this.backgroundTaskStore.start({
      kind: "git",
      label: `Rebasing onto ${onto}…`,
    });
    void this.runGitTask(taskId, "git rebase failed", () => gitRebaseAsync(stream.worktree_path, onto));
    return Promise.resolve({ taskId });
  }

  private async runGitTask(
    taskId: string,
    fallbackError: string,
    invoke: () => Promise<GitOpResult>,
  ): Promise<void> {
    try {
      const result = await invoke();
      if (result.ok) this.backgroundTaskStore.complete(taskId, result);
      else this.backgroundTaskStore.fail(taskId, result.stderr || fallbackError, result);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.backgroundTaskStore.fail(taskId, message);
    }
  }

  /**
   * Enumerate existing git worktrees that aren't already tracked as oxplow
   * streams. Powers the new-stream "adopt existing worktree" flow. The main
   * worktree (the project itself) is excluded since it's the primary stream.
   */
  /**
   * All git worktrees of this repo except the one backing `streamId`.
   * Used by the Git Dashboard's worktrees card so the user can see their
   * sibling streams' branches and merge from any of them. Unlike
   * `listAdoptableWorktrees`, this returns *every* sibling — including
   * worktrees already tracked as oxplow streams — because the dashboard
   * is a navigation/comparison surface, not the adoption flow.
   */
  listSiblingWorktrees(streamId: string): GitWorktreeEntry[] {
    const stream = this.resolveStream(streamId);
    const selfPath = stream.worktree_path;
    return listExistingWorktrees(this.projectDir).filter((wt) => wt.path !== selfPath);
  }

  listAdoptableWorktrees(): GitWorktreeEntry[] {
    const known = new Set(
      this.store.list().map((s) => s.worktree_path).filter((p): p is string => !!p),
    );
    return listExistingWorktrees(this.projectDir).filter((wt) => !wt.isMain && !known.has(wt.path));
  }

  getBranchChanges(streamId: string, baseRef?: string): BranchChanges & { resolvedBaseRef: string | null } {
    const stream = this.resolveStream(streamId);
    const resolvedBaseRef = baseRef?.trim() || detectBaseBranch(stream.worktree_path);
    if (!resolvedBaseRef) {
      return { baseRef: "", mergeBase: null, files: [], resolvedBaseRef: null };
    }
    const changes = listBranchChanges(stream.worktree_path, resolvedBaseRef);
    return { ...changes, resolvedBaseRef };
  }

  readFileAtRef(streamId: string, ref: string, path: string): { content: string | null } {
    const stream = this.resolveStream(streamId);
    return { content: readFileAtRef(stream.worktree_path, ref, path) };
  }

  getGitLog(streamId: string, options?: { limit?: number; all?: boolean }) {
    const stream = this.resolveStream(streamId);
    return getGitLog(stream.worktree_path, options);
  }

  getCommitDetail(streamId: string, sha: string): CommitDetail | null {
    const stream = this.resolveStream(streamId);
    return getCommitDetail(stream.worktree_path, sha);
  }

  getChangeScopes(streamId: string): ChangeScopes {
    const stream = this.resolveStream(streamId);
    return getChangeScopes(stream.worktree_path);
  }

  searchWorkspaceText(streamId: string, query: string, options?: { limit?: number }): TextSearchHit[] {
    const stream = this.resolveStream(streamId);
    return searchWorkspaceText(stream.worktree_path, query, options);
  }

  gitRestorePath(streamId: string, path: string): GitOpResult {
    const stream = this.resolveStream(streamId);
    return restorePath(stream.worktree_path, path);
  }

  gitAddPath(streamId: string, path: string): GitOpResult {
    const stream = this.resolveStream(streamId);
    return addPath(stream.worktree_path, path);
  }

  gitAppendToGitignore(streamId: string, path: string): GitOpResult {
    const stream = this.resolveStream(streamId);
    return appendToGitignore(stream.worktree_path, path);
  }

  gitPush(streamId: string, options?: Parameters<typeof gitPush>[1]): Promise<{ taskId: string }> {
    const stream = this.resolveStream(streamId);
    const branch = options?.branch ?? stream.branch ?? "HEAD";
    const remote = options?.remote ?? "origin";
    const taskId = this.backgroundTaskStore.start({
      kind: "git",
      label: `Pushing ${branch} to ${remote}…`,
    });
    void this.runGitTask(taskId, "git push failed", () => gitPushAsync(stream.worktree_path, options));
    return Promise.resolve({ taskId });
  }

  gitPull(streamId: string, options?: Parameters<typeof gitPull>[1]): Promise<{ taskId: string }> {
    const stream = this.resolveStream(streamId);
    const branch = options?.branch ?? stream.branch ?? "HEAD";
    const remote = options?.remote ?? "origin";
    const taskId = this.backgroundTaskStore.start({
      kind: "git",
      label: `Pulling ${branch} from ${remote}…`,
    });
    void this.runGitTask(taskId, "git pull failed", () => gitPullAsync(stream.worktree_path, options));
    return Promise.resolve({ taskId });
  }

  gitFetch(streamId: string, options?: { remote?: string; prune?: boolean; all?: boolean }): Promise<{ taskId: string }> {
    const stream = this.resolveStream(streamId);
    const remote = options?.remote ?? "origin";
    const taskId = this.backgroundTaskStore.start({
      kind: "git",
      label: `Fetching ${remote}…`,
    });
    void this.runGitTask(taskId, "git fetch failed", () => gitFetchAsync(stream.worktree_path, options));
    return Promise.resolve({ taskId });
  }

  gitCommitAll(streamId: string, message: string, options?: { includeUntracked?: boolean; paths?: string[] }): GitOpResult & { sha?: string } {
    const stream = this.resolveStream(streamId);
    return gitCommitAll(stream.worktree_path, message, options);
  }

  getAheadBehind(streamId: string, base: string, head?: string): { ahead: number; behind: number } {
    const stream = this.resolveStream(streamId);
    return getAheadBehind(stream.worktree_path, base, head);
  }

  getCommitsAheadOf(streamId: string, base: string, head: string, limit?: number): GitLogCommit[] {
    const stream = this.resolveStream(streamId);
    return getCommitsAheadOf(stream.worktree_path, base, head, limit);
  }

  listRecentRemoteBranches(streamId: string, limit?: number): RemoteBranchEntry[] {
    const stream = this.resolveStream(streamId);
    return listRecentRemoteBranches(stream.worktree_path, limit);
  }

  gitPushCurrentTo(streamId: string, remote: string, branch: string): Promise<{ taskId: string }> {
    const stream = this.resolveStream(streamId);
    const taskId = this.backgroundTaskStore.start({
      kind: "git",
      label: `Pushing HEAD to ${remote}/${branch}…`,
    });
    void this.runGitTask(taskId, "git push failed", () => gitPushCurrentToAsync(stream.worktree_path, remote, branch));
    return Promise.resolve({ taskId });
  }

  gitPullRemoteIntoCurrent(streamId: string, remote: string, branch: string): Promise<{ taskId: string }> {
    const stream = this.resolveStream(streamId);
    const taskId = this.backgroundTaskStore.start({
      kind: "git",
      label: `Pulling ${remote}/${branch} into current…`,
    });
    void this.runGitTask(taskId, "git pull failed", () => gitPullRemoteIntoCurrent(stream.worktree_path, remote, branch));
    return Promise.resolve({ taskId });
  }

  /** Lookup a single background task by id. Used by the renderer to read
   *  `task.result` after awaiting a kickoff IPC. */
  getBackgroundTask(id: string): import("./background-task-store.js").BackgroundTask | null {
    return this.backgroundTaskStore.get(id);
  }

  listFileCommits(streamId: string, path: string, limit?: number): GitLogCommit[] {
    const stream = this.resolveStream(streamId);
    return listFileCommits(stream.worktree_path, path, limit);
  }

  gitBlame(streamId: string, path: string) {
    const stream = this.resolveStream(streamId);
    return gitBlame(stream.worktree_path, path);
  }

  /**
   * Per-line blame combining oxplow work-item efforts (authoritative) with
   * git blame (fallback). See `src/electron/local-blame.ts` for the
   * algorithm and `.context/editor-and-monaco.md` for the UI wiring.
   */
  localBlame(streamId: string, path: string): import("./local-blame.js").LocalBlameEntry[] {
    const stream = this.resolveStream(streamId);
    let diskText: string;
    try {
      diskText = readWorkspaceFile(stream.worktree_path, path).content;
    } catch {
      return [];
    }
    return computeLocalBlame({
      effortStore: this.effortStore,
      snapshotStore: this.snapshotStore,
      path,
      diskText,
      gitBlame: () => gitBlame(stream.worktree_path, path),
    });
  }

  listAllRefs(streamId: string): RefOption[] {
    const stream = this.resolveStream(streamId);
    return listAllRefs(stream.worktree_path);
  }

  getWorkspaceContext() {
    // Re-read rather than returning the cache — callers (IPC) expect a fresh
    // answer; the cache exists only to debounce event publishing.
    const gitEnabled = isGitRepo(this.projectDir);
    this.gitEnabledCached = gitEnabled;
    return { gitEnabled };
  }

  /**
   * Switch the branch checked out in `streamId`'s worktree. Works for both
   * primary (worktree_path === projectDir) and worktree streams. Lets git's
   * own errors (dirty tree, missing branch, already-checked-out-elsewhere)
   * propagate unchanged.
   */
  checkoutStreamBranch(streamId: string, branch: string): Stream {
    const stream = this.resolveStream(streamId);
    const previousBranch = stream.branch;
    const previousTitle = stream.title;
    checkoutBranch(stream.worktree_path, branch);
    const detected = detectCurrentBranch(stream.worktree_path) ?? branch;
    const branchRef = `refs/heads/${detected}`;
    let updated = this.store.setStreamBranch(stream.id, detected, branchRef);
    // Keep the tab label in sync for worktree streams whose title has always
    // tracked their branch. Primary streams are exempt — their title is the
    // repo basename and never changes.
    if (updated.kind === "worktree" && previousTitle === previousBranch && previousTitle !== detected) {
      updated = this.store.update(updated.id, (current) => ({ ...current, title: detected }));
    }
    this.logger.info("checked out branch", { streamId: stream.id, branch: detected });
    return updated;
  }

  /**
   * React to an external HEAD move (`git checkout` from the terminal, a
   * pull that moved the branch, etc.) by refreshing the stream's recorded
   * branch. Called by the git-refs watcher on any ref/HEAD event.
   */
  private maybeSyncStreamBranch(streamId: string): void {
    const stream = this.store.get(streamId);
    if (!stream) return;
    const detected = detectCurrentBranch(stream.worktree_path);
    if (!detected || detected === stream.branch) return;
    try {
      this.store.setStreamBranch(stream.id, detected, `refs/heads/${detected}`);
      this.logger.info("stream branch updated from HEAD", { streamId: stream.id, branch: detected });
    } catch (error) {
      this.logger.warn("failed to sync stream branch from HEAD", {
        streamId: stream.id,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  createStream(body:
    | { title: string; summary?: string; source: "existing"; ref: string }
    | { title: string; summary?: string; source: "new"; branch: string; startPointRef: string }
    | { title: string; summary?: string; source: "worktree"; worktreePath: string },
  ): Stream {
    if (!isGitRepo(this.projectDir)) {
      throw new Error("git functionality is disabled for this workspace root");
    }
    if (!body.title.trim()) {
      throw new Error("title is required");
    }
    const title = body.title.trim();
    const summary = body.summary?.trim() ?? "";
    let stream: Stream;
    if (body.source === "existing") {
      const branch = listBranches(this.projectDir).find((candidate) => candidate.ref === body.ref);
      if (!branch) throw new Error(`unknown branch ref: ${body.ref}`);
      const localBranch = branch.kind === "local" ? branch.name : localBranchName(branch.name);
      const existing = this.store.findByBranch(localBranch);
      if (existing) {
        this.store.setCurrentStreamId(existing.id);
        return existing;
      }
      const worktreePath = streamWorktreePath(this.projectDir, localBranch);
      ensureWorktree(
        this.projectDir,
        worktreePath,
        branch.kind === "local"
          ? { kind: "existing-local", branch: localBranch }
          : { kind: "existing-remote", branch: localBranch, remoteRef: branch.name },
      );
      stream = this.store.create({
        title,
        summary,
        branch: localBranch,
        branchRef: branch.ref,
        branchSource: branch.kind,
        worktreePath,
        projectBase: this.projectBase,
      });
    } else if (body.source === "new") {
      const branchName = body.branch.trim();
      if (!branchName) throw new Error("branch is required");
      if (!body.startPointRef.trim()) throw new Error("startPointRef is required");
      const existing = this.store.findByBranch(branchName);
      if (existing) {
        this.store.setCurrentStreamId(existing.id);
        return existing;
      }
      const worktreePath = streamWorktreePath(this.projectDir, branchName);
      ensureWorktree(this.projectDir, worktreePath, {
        kind: "new",
        branch: branchName,
        startPoint: body.startPointRef,
      });
      stream = this.store.create({
        title,
        summary,
        branch: branchName,
        branchRef: body.startPointRef,
        branchSource: "new",
        worktreePath,
        projectBase: this.projectBase,
      });
    } else {
      // Adopt an existing git worktree — skip ensureWorktree, just detect the
      // branch and register the stream pointing at the user-supplied path.
      const worktreePath = body.worktreePath;
      if (!worktreePath.trim()) throw new Error("worktreePath is required");
      if (!isGitRepo(worktreePath)) throw new Error(`not a git worktree: ${worktreePath}`);
      const already = this.store.list().find((s) => s.worktree_path === worktreePath);
      if (already) {
        throw new Error(`worktree is already adopted as stream "${already.title}"`);
      }
      const branchName = detectCurrentBranch(worktreePath);
      if (!branchName) throw new Error(`worktree has a detached HEAD: ${worktreePath}`);
      const existing = this.store.findByBranch(branchName);
      if (existing) {
        throw new Error(`branch "${branchName}" is already mapped to stream "${existing.title}"`);
      }
      stream = this.store.create({
        title,
        summary,
        branch: branchName,
        branchRef: `refs/heads/${branchName}`,
        branchSource: "local",
        worktreePath,
        projectBase: this.projectBase,
      });
    }
    this.workspaceWatchers.ensureWatching(stream);
    this.gitRefsWatchers.ensureWatching(stream);
    this.threadStore.ensureStream(stream);
    this.store.setCurrentStreamId(stream.id);
    return stream;
  }

  getThreadState(streamId: string): ThreadState {
    const stream = this.resolveStream(streamId);
    return this.threadStore.ensureStream(stream);
  }

  createThread(streamId: string, title: string): ThreadState {
    const stream = this.resolveStream(streamId);
    if (!title.trim()) throw new Error("thread title is required");
    return this.threadStore.create(stream, { title });
  }

  reorderThread(streamId: string, threadId: string, targetIndex: number): ThreadState {
    this.resolveStream(streamId);
    return this.threadStore.reorder(streamId, threadId, targetIndex);
  }

  reorderThreads(streamId: string, orderedThreadIds: string[]): void {
    this.resolveStream(streamId);
    this.threadStore.reorderThreads(streamId, orderedThreadIds);
  }

  reorderStreams(orderedStreamIds: string[]): void {
    this.store.reorderStreams(orderedStreamIds);
  }

  selectThread(streamId: string, threadId: string): ThreadState {
    this.resolveStream(streamId);
    return this.threadStore.select(streamId, threadId);
  }

  promoteThread(streamId: string, threadId: string): ThreadState {
    this.resolveStream(streamId);
    return this.threadStore.promote(streamId, threadId);
  }

  closeThread(streamId: string, threadId: string): ThreadState {
    this.resolveStream(streamId);
    return this.threadStore.close(streamId, threadId);
  }

  reopenThread(streamId: string, threadId: string): ThreadState {
    this.resolveStream(streamId);
    return this.threadStore.reopen(streamId, threadId);
  }

  listClosedThreads(streamId: string): Thread[] {
    this.resolveStream(streamId);
    return this.threadStore.listClosed(streamId);
  }

  renameThread(streamId: string, threadId: string, title: string): Thread {
    this.resolveStream(streamId);
    return this.threadStore.rename(streamId, threadId, title);
  }

  setStreamPrompt(streamId: string, prompt: string | null): Stream[] {
    return this.store.setStreamPrompt(streamId, prompt);
  }

  setThreadPrompt(streamId: string, threadId: string, prompt: string | null): Thread[] {
    this.resolveThread(streamId, threadId);
    return this.threadStore.setThreadPrompt(threadId, prompt);
  }

  listWorkspaceEntries(streamId: string, path = "") {
    const stream = this.resolveStream(streamId);
    const statuses = listGitStatuses(stream.worktree_path);
    return listWorkspaceEntries(stream.worktree_path, path, statuses);
  }

  listWorkspaceFiles(streamId: string) {
    const stream = this.resolveStream(streamId);
    const statuses = listGitStatuses(stream.worktree_path);
    return {
      files: listWorkspaceFiles(stream.worktree_path, statuses),
      summary: summarizeGitStatuses(statuses),
    };
  }

  readWorkspaceFile(streamId: string, path: string) {
    return readWorkspaceFile(this.resolveStream(streamId).worktree_path, path);
  }

  writeWorkspaceFile(streamId: string, path: string, content: string) {
    const stream = this.resolveStream(streamId);
    this.stampUiWrite(path);
    const saved = writeWorkspaceFile(stream.worktree_path, path, content);
    this.stampUiWrite(saved.path);
    this.workspaceWatchers.notify(stream.id, "updated", saved.path);
    return saved;
  }

  createWorkspaceFile(streamId: string, path: string, content = "") {
    const stream = this.resolveStream(streamId);
    this.stampUiWrite(path);
    const created = createWorkspaceFile(stream.worktree_path, path, content);
    this.stampUiWrite(created.path);
    this.workspaceWatchers.notify(stream.id, "created", created.path);
    return created;
  }

  createWorkspaceDirectory(streamId: string, path: string) {
    const stream = this.resolveStream(streamId);
    this.stampUiWrite(path);
    const created = createWorkspaceDirectory(stream.worktree_path, path);
    this.stampUiWrite(created.path);
    this.workspaceWatchers.notify(stream.id, "created", created.path);
    return created;
  }

  renameWorkspacePath(streamId: string, fromPath: string, toPath: string) {
    const stream = this.resolveStream(streamId);
    this.stampUiWrite(fromPath);
    this.stampUiWrite(toPath);
    const renamed = renameWorkspacePath(stream.worktree_path, fromPath, toPath);
    this.stampUiWrite(renamed.fromPath);
    this.stampUiWrite(renamed.toPath);
    this.workspaceWatchers.notify(stream.id, "deleted", renamed.fromPath);
    this.workspaceWatchers.notify(stream.id, "created", renamed.toPath);
    return renamed;
  }

  deleteWorkspacePath(streamId: string, path: string) {
    const stream = this.resolveStream(streamId);
    this.stampUiWrite(path);
    const deleted = deleteWorkspacePath(stream.worktree_path, path);
    this.stampUiWrite(deleted.path);
    this.workspaceWatchers.notify(stream.id, "deleted", deleted.path);
    return deleted;
  }

  listHookEvents(streamId?: string) {
    return this.hookEvents.list(streamId);
  }

  async updateEditorFocus(payload: EditorFocusPayload): Promise<void> {
    const { streamId, ...rest } = payload;
    if (!streamId) return;
    const state: EditorFocusState = {
      activeFile: rest.activeFile,
      caret: rest.caret,
      selection: rest.selection,
      openFiles: rest.openFiles ?? [],
      updatedAt: new Date().toISOString(),
    };
    this.editorFocusStore.set(streamId, state);
  }

  async logUi(payload: UiLogPayload): Promise<void> {
    const logger = createUiClientLogger(this.projectDir, payload.clientId);
    logger[parseLogLevel(payload.level)](payload.message, {
      ...(payload.context ?? {}),
      ...(payload.timestamp ? { clientTime: payload.timestamp } : {}),
    });
  }

  ping() {
    return true;
  }

  openTerminalSession(
    paneTarget: string,
    cols: number,
    rows: number,
    mode: "direct" | "tmux",
    onSend: (sessionId: string, message: string) => void,
  ): string {
    const thread = this.threadStore.findByPane(paneTarget);
    if (!thread) {
      throw new Error(`unknown pane target: ${paneTarget}`);
    }
    const stream = this.resolveStream(thread.stream_id);
    const paneLogger = this.logger.child({
      streamId: stream.id,
      threadId: thread.id,
      paneTarget,
    });
    const agentCommand = this.getAgentCommand(stream, thread);
    if (mode === "tmux") {
      // Use a resume-less variant as the launcher identity so reconnecting to
      // a live agent whose resume id has since changed doesn't look like a
      // config change and trigger a respawn.
      const signatureSource = this.getAgentCommand(stream, thread, { withoutResume: true });
      ensureAgentPane(
        thread.pane_target,
        stream.worktree_path,
        cols,
        rows,
        agentCommand,
        { signatureSource, logger: paneLogger },
      );
    }

    const sessionId = randomUUID();
    const socket = new RuntimeSocket((message) => onSend(sessionId, message));
    socket.on("close", () => {
      this.terminalSessions.delete(sessionId);
      this.terminalSessionThread.delete(sessionId);
    });
    this.terminalSessionThread.set(sessionId, { streamId: stream.id, threadId: thread.id });
    if (mode === "tmux") {
      attachPane(socket, thread.pane_target, cols, rows, paneLogger.child({ subsystem: "pty-bridge", mode }));
    } else {
      // Direct-mode agent PTYs live in the runtime and persist across
      // UI attach/detach. Switching threads or streams detaches the socket
      // but leaves the Claude process running so the user can return to an
      // in-progress agent without killing and resuming it.
      const agentPty = this.agentPtyStore.ensure(
        thread.id,
        { command: agentCommand, cwd: stream.worktree_path, cols, rows },
        paneLogger.child({ subsystem: "agent-pty" }),
      );
      agentPty.attach(socket, cols, rows);
    }
    this.terminalSessions.set(sessionId, socket);
    return sessionId;
  }

  sendTerminalMessage(sessionId: string, message: string): void {
    const socket = this.terminalSessions.get(sessionId);
    if (!socket) throw new Error(`unknown terminal session: ${sessionId}`);
    // Detect user-driven interrupts (Escape / Ctrl-C). Claude Code
    // doesn't fire a Stop hook on Esc cancellation, so without a
    // synthetic signal the tab icon stays "thinking" until the next
    // prompt. Synthesize a meta `Interrupt` hook event keyed to this
    // session's thread; the agent-status reducer treats it as a forced
    // reset of "working" → "waiting".
    if (terminalInputIsInterrupt(message)) {
      const ctx = this.terminalSessionThread.get(sessionId);
      if (ctx && this.agentStatusByThread.get(ctx.threadId) === "working") {
        ingestHookPayload(this.hookEvents, "Interrupt", {}, {
          streamId: ctx.streamId,
          threadId: ctx.threadId,
        });
        this.recomputeAgentStatus(ctx.streamId, ctx.threadId);
      }
    }
    socket.emit("message", message);
  }

  closeTerminalSession(sessionId: string): void {
    const socket = this.terminalSessions.get(sessionId);
    if (!socket) return;
    this.terminalSessions.delete(sessionId);
    this.terminalSessionThread.delete(sessionId);
    socket.close();
  }

  async openLspClient(streamId: string, languageId: string, onSend: (clientId: string, message: string) => void): Promise<string> {
    const stream = this.resolveStream(streamId);
    const clientId = randomUUID();
    const socket = new RuntimeSocket((message) => onSend(clientId, message));
    socket.on("close", () => {
      this.lspClients.delete(clientId);
    });
    this.lspClients.set(clientId, socket);
    await this.lspManager.attachClient(socket, stream, languageId);
    return clientId;
  }

  sendLspMessage(clientId: string, message: string): void {
    const socket = this.lspClients.get(clientId);
    if (!socket) throw new Error(`unknown lsp client: ${clientId}`);
    socket.emit("message", message);
  }

  closeLspClient(clientId: string): void {
    const socket = this.lspClients.get(clientId);
    if (!socket) return;
    this.lspClients.delete(clientId);
    socket.close();
  }

  private resolveStream(streamId: string | null | undefined): Stream {
    const id = streamId ?? this.store.getCurrentStreamId();
    if (!id) throw new Error("no current stream");
    const stream = this.store.get(id);
    if (!stream) throw new Error(`unknown stream: ${id}`);
    return stream;
  }

  private resolveThread(streamId: string, threadId: string): Thread {
    this.resolveStream(streamId);
    const thread = this.threadStore.getThread(streamId, threadId);
    if (!thread) throw new Error(`unknown thread: ${threadId}`);
    return thread;
  }

  // threadIds are globally unique, so a lookup by id alone is enough. MCP
  // tools use this when the caller omitted streamId (or passed one that
  // drifted out of sync with the UI's current stream); the agent's session
  // prompt shouldn't need to stay perfectly aligned with whatever stream
  // the user is viewing.
  private resolveThreadById(threadId: string): Thread {
    const thread = this.threadStore.findById(threadId);
    if (!thread) throw new Error(`unknown thread: ${threadId}`);
    return thread;
  }

  // Build a <session-context> additionalContext block reflecting LIVE state
  // (as opposed to the frozen snapshot in the agent's system prompt). Called
  // on every UserPromptSubmit — see handleHookEnvelope. Returns empty string
  // when the envelope lacks enough to resolve a thread.
  private buildRefreshedSessionContext(
    envelopeThreadId: string | null,
    streamId: string,
    sessionId: string | undefined,
  ): string {
    void streamId;
    const thread = envelopeThreadId ? this.threadStore.findById(envelopeThreadId) : null;
    if (!thread) return "";
    const stream = this.store.get(thread.stream_id);
    if (!stream) return "";
    const threadState = this.threadStore.list(stream.id);
    const activeThread = threadState.threads.find((b) => b.id === threadState.activeThreadId) ?? null;
    // Stash (once) the role this thread had when Claude's session id was first
    // observed, so a later promotion/demotion surfaces as a ROLE CHANGE banner
    // rather than a subtle one-line diff.
    const currentRole: "writer" | "read-only" =
      activeThread && activeThread.id !== thread.id ? "read-only" : "writer";
    let initialRole: "writer" | "read-only" | undefined;
    if (sessionId) {
      if (!this.initialRoleBySessionId.has(sessionId)) {
        this.initialRoleBySessionId.set(sessionId, currentRole);
      }
      initialRole = this.initialRoleBySessionId.get(sessionId);
    }
    return buildSessionContextBlock({
      stream,
      thread,
      activeThread,
      initialRole,
    });
  }

  private buildRecentDoneReminder(threadId: string): string {
    return buildRecentDoneReminder(this.workItemStore.listItems(threadId), Date.now());
  }

  private buildPriorPromptInProgressReminder(threadId: string): string {
    return buildPriorPromptInProgressReminder(this.workItemStore.listItems(threadId), Date.now());
  }

  private resolveActiveThreadForPrompt(streamId: string): Thread | null {
    const activeId = this.threadStore.list(streamId).activeThreadId;
    if (!activeId) return null;
    return this.threadStore.getThread(streamId, activeId) ?? null;
  }

  private getAgentCommand(stream: Stream, thread: Thread, opts: { withoutResume?: boolean } = {}): string {
    const resumeSessionId = opts.withoutResume ? "" : thread.resume_session_id;
    if (this.config.agent === "claude") {
      if (!this.mcp) throw new Error("mcp server not started");
      // One Claude plugin per runtime (the MCP port + hook URL are stable for
      // the process's lifetime). Plugin hook JSON references env vars, so
      // per-thread identity flows in at exec time without re-writing files.
      if (!this.electronPlugin) {
        this.electronPlugin = createElectronPlugin({
          projectDir: this.projectDir,
          hookUrl: this.mcp.hookUrl,
        });
        this.logger.info("wrote claude plugin", {
          pluginDir: this.electronPlugin.pluginDir,
        });
      }
      return buildAgentCommandForSession(
        this.config.agent,
        stream.worktree_path,
        resumeSessionId,
        {
          pluginDir: this.electronPlugin.pluginDir,
          allowedTools: ["mcp__oxplow__*"],
          appendSystemPrompt: buildThreadAgentPrompt(
            stream,
            thread,
            this.config.agentPromptAppend,
            this.resolveActiveThreadForPrompt(stream.id),
          ),
          mcpConfig: buildThreadMcpConfig(this.mcp),
          env: {
            OXPLOW_STREAM_ID: stream.id,
            OXPLOW_THREAD_ID: thread.id,
            OXPLOW_HOOK_TOKEN: this.mcp.authToken,
          },
        },
      );
    }
    return buildAgentCommandForSession(
      this.config.agent,
      stream.worktree_path,
      resumeSessionId,
    );
  }

  /**
   * Called by the `oxplow__await_user` MCP handler. Records that the
   * agent is explicitly waiting on the user. The next Stop hook reads
   * this and suppresses every directive so the agent doesn't march onto
   * the next queue item past the open question. Cleared on
   * UserPromptSubmit when the user's reply lands.
   */
  markAwaitingUser(threadId: string, question: string): void {
    this.awaitingUserByThread.set(threadId, { question, setAt: Date.now() });
  }

  /**
   * Called by every work-item-mutating MCP handler (create / update /
   * complete / file_epic / transition / dispatch). Marks the per-turn
   * "agent filed something" flag the Stop-hook filing-enforcement branch
   * reads. Cleared on UserPromptSubmit.
   */
  markFiledThisTurn(threadId: string): void {
    this.filedThisTurnByThread.add(threadId);
  }

  /**
   * Stricter sibling of `markFiledThisTurn`: called only by
   * `create_work_item` / `file_epic_with_children` when the new row
   * landed at `ready` (no in_progress claim). Drives the Stop-hook
   * "filed but didn't ship" advisory branch.
   */
  markFiledReadyThisTurn(threadId: string): void {
    this.filedReadyThisTurnByThread.add(threadId);
  }

  /**
   * fork_thread MCP tool implementation. Creates a new thread on the
   * same stream as `sourceThreadId`, seeds it with a `note`-kind work
   * item carrying `summary` (no schema change — avoids a new table),
   * and optionally moves the listed ready/blocked work items across.
   * Returns the new thread id.
   *
   * Validation errors (non-existent thread, non-movable item statuses)
   * throw before any DB mutation so the caller sees the problem.
   */
  forkThread(input: {
    sourceThreadId: string;
    title: string;
    summary: string;
    moveItemIds?: string[];
  }): { newThreadId: string } {
    const source = this.threadStore.findById(input.sourceThreadId);
    if (!source) throw new Error(`unknown thread: ${input.sourceThreadId}`);
    const title = input.title?.trim();
    if (!title) throw new Error("fork_thread: `title` is required");
    const stream = this.store.get(source.stream_id);
    if (!stream) throw new Error(`source thread has no stream: ${input.sourceThreadId}`);
    const movableStatuses = new Set(["ready", "blocked"]);
    const toMove = (input.moveItemIds ?? [])
      .map((id) => this.workItemStore.getItem(input.sourceThreadId, id));
    const offenders: Array<{ id: string; status: string }> = [];
    for (const [i, item] of toMove.entries()) {
      const id = (input.moveItemIds ?? [])[i]!;
      if (!item) throw new Error(`fork_thread: unknown item: ${id}`);
      if (item.thread_id !== input.sourceThreadId) {
        throw new Error(`fork_thread: item ${id} belongs to a different thread`);
      }
      if (!movableStatuses.has(item.status)) {
        offenders.push({ id, status: item.status });
      }
    }
    if (offenders.length > 0) {
      const desc = offenders.map((o) => `${o.id} (${o.status})`).join(", ");
      throw new Error(`fork_thread: cannot move items not in ready/blocked: ${desc}`);
    }
    const beforeIds = new Set(this.threadStore.list(stream.id).threads.map((t) => t.id));
    const afterState = this.threadStore.create(stream, { title });
    const created = afterState.threads.find((t) => !beforeIds.has(t.id));
    if (!created) throw new Error("fork_thread: failed to create new thread");
    const newThreadId = created.id;
    // Seed note on the new thread.
    this.workItemStore.createItem({
      threadId: newThreadId,
      kind: "note",
      title: "Context from fork",
      description: input.summary,
      createdBy: "agent",
      actorId: "fork_thread",
    });
    // Move items across. For each moved item, carry its last 3 notes over
    // as fresh rows on the same item id so the user landing in the new
    // thread sees recent rationale/decisions rather than a bare title.
    // Copies are additive — source rows are never touched (see
    // WorkItemStore.copyLastItemNotes).
    for (const id of input.moveItemIds ?? []) {
      this.workItemStore.moveItemToThread(input.sourceThreadId, id, newThreadId, "agent", "fork_thread");
      try {
        this.workItemStore.copyLastItemNotes(id, 3);
      } catch (err) {
        this.logger.warn("fork_thread: copyLastItemNotes failed", {
          itemId: id,
          error: err instanceof Error ? err.message : String(err),
        });
      }
    }
    return { newThreadId };
  }

  private handleHookEnvelope(envelope: HookEnvelope): { body?: unknown } | void {
    const streamId = envelope.streamId;
    if (!streamId) return;
    this.hookEventsSeen.add(envelope.event);
    const pane: PaneKind | undefined = envelope.pane === "working" || envelope.pane === "talking"
      ? envelope.pane
      : undefined;
    const stored = ingestHookPayload(this.hookEvents, envelope.event, envelope.payload, {
      streamId,
      threadId: envelope.threadId,
      pane,
    });
    if (envelope.threadId && this.store.get(streamId)) {
      const thread = this.threadStore.findById(envelope.threadId);
      const update = decideResumeUpdate(
        thread?.resume_session_id ?? "",
        stored.normalized.sessionId,
      );
      if (update) {
        this.threadStore.updateResume(streamId, envelope.threadId, update.sessionId);
      }
      if (envelope.event === "PostToolUse") {
        this.handlePostToolUseDirty(envelope);
        const payload = (envelope.payload ?? {}) as { tool_name?: unknown; tool_input?: unknown };
        const activityToolName = typeof payload.tool_name === "string" ? payload.tool_name : "";
        if (activityToolName && envelope.threadId && isActivityTool(activityToolName, payload.tool_input)) {
          this.turnActivityByThread.set(envelope.threadId, true);
        }
        if (activityToolName && envelope.threadId) {
          if (isWriteIntentTool(activityToolName, payload.tool_input)) {
            const cur = this.turnWriteCountByThread.get(envelope.threadId) ?? 0;
            this.turnWriteCountByThread.set(envelope.threadId, cur + 1);
          }
        }
        // Match a PreToolUse for `Task` (subagent dispatch) — decrement
        // the per-thread pending-subagent counter so the Stop-hook
        // suppression lifts once the subagent returns.
        const toolName = typeof (envelope.payload as { tool_name?: unknown })?.tool_name === "string"
          ? (envelope.payload as { tool_name: string }).tool_name
          : "";
        if (toolName === "Task" && envelope.threadId) {
          const cur = this.pendingSubagentsByThread.get(envelope.threadId) ?? 0;
          if (cur <= 1) this.pendingSubagentsByThread.delete(envelope.threadId);
          else this.pendingSubagentsByThread.set(envelope.threadId, cur - 1);
        }
      }
    }
    if (envelope.event === "PreToolUse" && envelope.threadId) {
      const toolName0 = typeof (envelope.payload as { tool_name?: unknown })?.tool_name === "string"
        ? (envelope.payload as { tool_name: string }).tool_name
        : "";
      if (toolName0 === "Task") {
        const cur = this.pendingSubagentsByThread.get(envelope.threadId) ?? 0;
        this.pendingSubagentsByThread.set(envelope.threadId, cur + 1);
      }
    }
    if (envelope.event === "PreToolUse" && envelope.threadId) {
      // Fresh read of thread.status — promoting another thread to writer takes
      // effect on the next tool call without restarting any agent.
      const thread = this.threadStore.findById(envelope.threadId);
      const toolName = typeof (envelope.payload as { tool_name?: unknown })?.tool_name === "string"
        ? (envelope.payload as { tool_name: string }).tool_name
        : "";
      const deny = buildWriteGuardResponse(thread, toolName, {
        projectDir: this.projectDir,
        toolInput: (envelope.payload as { tool_input?: unknown })?.tool_input,
      });
      if (deny) return { body: deny };
      // Filing-enforcement guard: block Edit/Write/MultiEdit/NotebookEdit
      // when the writer thread has no in_progress item to claim the
      // change. Catches the misread at the moment the agent can act on
      // it (file/transition → re-edit) instead of at end-of-turn after
      // the write has already shipped. A `ready`-status filing call
      // alone does NOT satisfy the guard — `ready` is backlog, only
      // `in_progress` is a commitment to ship.
      const inProgressOpen = this.workItemStore
        .listItems(envelope.threadId)
        .some((item) => item.status === "in_progress");
      const toolInputFilePath = extractEditedFilePath(
        (envelope.payload as { tool_input?: unknown })?.tool_input,
      );
      const filingDeny = buildFilingEnforcementPreToolDeny({
        thread,
        toolName,
        hasInProgressItem: inProgressOpen,
        filePath: toolInputFilePath,
      });
      if (filingDeny) return { body: filingDeny };
    }
    if (envelope.event === "UserPromptSubmit") {
      // Seed the per-turn activity flag — flips to true on the first
      // qualifying PostToolUse. Read on Stop to suppress the full
      // directive pipeline for pure Q&A turns.
      if (envelope.threadId) {
        this.turnActivityByThread.set(envelope.threadId, false);
        // Reset per-turn write counter so each turn's filing-enforcement
        // check starts clean.
        this.turnWriteCountByThread.set(envelope.threadId, 0);
        // The user replied → clear the "agent is awaiting user" gate so
        // the next Stop runs the directive pipeline normally. Also reset
        // the per-turn filing flag so each turn enforces filing afresh.
        this.awaitingUserByThread.delete(envelope.threadId);
        this.filedThisTurnByThread.delete(envelope.threadId);
        this.filedReadyThisTurnByThread.delete(envelope.threadId);
        this.filedButDidntShipFiredByThread.delete(envelope.threadId);
      }
      const focusContext = formatEditorFocusForAgent(this.editorFocusStore.get(streamId));
      // If an agent-authored done item was closed on this thread very
      // recently, there's a strong chance this new prompt is either a
      // redo on it or a follow-up concern. Inject a reminder pointing
      // at the item so the agent knows to reopen (update_work_item →
      // in_progress) rather than silently expand scope or file a
      // duplicate "Fix …" task.
      const redoReminder = envelope.threadId
        ? this.buildRecentDoneReminder(envelope.threadId)
        : "";
      // When a new user prompt arrives while an in_progress item from a
      // prior prompt is still open, nudge the agent to either file a
      // fresh row (new concern) or explicitly reopen the existing one
      // (fix/redo). Catches the failure mode where mid-turn user
      // messages get silently bundled into whatever item happened to be
      // open. Pairs with the recent-done reminder above — that fires
      // when the prior item already closed; this one fires when it's
      // still running.
      const priorPromptInProgressReminder = envelope.threadId
        ? this.buildPriorPromptInProgressReminder(envelope.threadId)
        : "";
      // Wiki-capture hint: when the prompt looks like exploration /
      // synthesis, nudge the agent up-front to plan on writing a wiki
      // note. Replaces the old Stop-hook wiki-capture directive, which
      // fired post-hoc after the answer had already gone to chat with
      // no durable home. Returns null for non-exploration prompts —
      // those pay no token cost.
      const promptText = typeof (envelope.payload as { prompt?: unknown })?.prompt === "string"
        ? (envelope.payload as { prompt: string }).prompt
        : "";
      const wikiCaptureHint = buildWikiCaptureHint(promptText) ?? "";
      // Ready-match nudge: if exactly one ready item on this thread
      // plausibly matches the new prompt, point at it so the agent
      // flips the existing row to in_progress instead of filing a
      // duplicate task. Conservative — silent on ambiguity. Skipped
      // entirely when there's no thread (system events).
      const readyMatchReminder = envelope.threadId
        ? buildReadyMatchReminder(this.workItemStore.listItems(envelope.threadId), promptText)
        : "";
      // Re-inject the session context each turn — the agent's system-prompt
      // SESSION CONTEXT line is frozen at launch, but the UI's active /
      // selected thread can flip mid-session. Reading the live state here
      // keeps the agent pointed at the right ids without a user-visible
      // prompt edit. Skip emission when the block is identical to what we
      // already sent on the same Claude session — the agent's prompt cache
      // still holds the prior value, so re-sending is pure overhead.
      let sessionContext = "";
      if (this.config.injectSessionContext) {
        const candidate = this.buildRefreshedSessionContext(envelope.threadId ?? null, streamId, stored.normalized.sessionId);
        const sessionKey = stored.normalized.sessionId ?? "";
        if (candidate && sessionKey) {
          if (this.lastSessionContextBySessionId.get(sessionKey) !== candidate) {
            sessionContext = candidate;
            this.lastSessionContextBySessionId.set(sessionKey, candidate);
          }
        } else {
          sessionContext = candidate;
        }
      }
      // Wiki-capture goes FIRST so the imperative frame lands before the
      // agent reads the rest of the reminder pile. When it trails the
      // session-context / focus / redo blocks the agent treats it as a
      // post-answer afterthought and skips the note write; leading with
      // it sets the turn's frame as "synthesis = write a note".
      const additionalContext = [wikiCaptureHint, sessionContext, focusContext, redoReminder, priorPromptInProgressReminder, readyMatchReminder].filter(Boolean).join("\n\n");
      if (additionalContext) {
        return {
          body: {
            hookSpecificOutput: {
              hookEventName: "UserPromptSubmit",
              additionalContext,
            },
          },
        };
      }
    }
    if (envelope.event === "Stop" && envelope.threadId) {
      const directive = this.computeStopDirective(envelope.threadId);
      this.reportHookHealthIfNeeded();
      if (directive) return { body: directive };
    }
  }

  // Warn once per process about registered hooks that never delivered, so
  // a silent-drop (SessionStart is the known culprit; Claude Code's debug
  // log says "HTTP hooks are not supported for SessionStart") doesn't get
  // lost. Runs at first Stop — by then a "full" turn has happened and the
  // hook set is a useful signal. Paths that legitimately don't fire
  // (PreToolUse / PostToolUse / Notification when the turn didn't exercise
  // them) are grouped into one informational line rather than warn'd per
  // event, so the output is readable.
  private reportHookHealthIfNeeded(): void {
    if (this.hookHealthReported) return;
    this.hookHealthReported = true;
    for (const report of describeHookHealth(HOOK_EVENTS, this.hookEventsSeen)) {
      this.logger.warn(report.message, { event: report.event, hint: report.hint });
    }
  }

  /**
   * Stop-hook pipeline entry point. Snapshots the relevant stores, hands
   * the snapshot to the pure `decideStopDirective`, applies any returned
   * side effects (e.g. recording the audit signature), and returns the
   * hook body for Claude. The decision logic itself lives in
   * `src/electron/stop-hook-pipeline.ts` so each branch is unit-testable
   * without spinning up a runtime.
   */
  private computeStopDirective(threadId: string): Record<string, unknown> | null {
    const thread = this.threadStore.findById(threadId);
    // Consume the activity flag for this turn (once per Stop). Absent =
    // unknown — pipeline treats undefined as "don't suppress" so older
    // tests / edge cases stay stable.
    const turnHadActivity = this.turnActivityByThread.get(threadId);
    this.turnActivityByThread.delete(threadId);
    // Per-turn write count drives `turnHadWrites` for the filing-
    // enforcement Stop branch. Clear together with the activity flag so
    // it doesn't leak into the next turn.
    const writeCount = this.turnWriteCountByThread.get(threadId) ?? 0;
    this.turnWriteCountByThread.delete(threadId);
    const turnHadWrites = writeCount > 0;
    // Consume the per-turn filing flag (once per Stop) so each turn
    // enforces filing afresh. Don't `delete` here — the flag is also
    // cleared on UserPromptSubmit; deleting on Stop would silently let
    // the next Stop in the same prompt-gap pass without a filing check.
    const turnHadFiling = this.filedThisTurnByThread.has(threadId);
    const turnFiledReadyItem = this.filedReadyThisTurnByThread.has(threadId);
    const filedButDidntShipFired = this.filedButDidntShipFiredByThread.has(threadId);
    const awaitingUser = this.awaitingUserByThread.has(threadId);
    const snapshot: ThreadSnapshot = {
      thread,
      workItems: this.workItemStore.listItems(threadId),
      turnHadActivity,
      subagentInFlight: (this.pendingSubagentsByThread.get(threadId) ?? 0) > 0,
      lastInProgressAuditSignature: this.lastAuditSignatureByThread.get(threadId),
      turnHadWrites,
      turnHadFiling,
      turnFiledReadyItem,
      filedButDidntShipFired,
      awaitingUser,
    };
    const outcome = decideStopDirective(snapshot, {
      buildInProgressAuditReason: buildInProgressAuditStopReason,
      buildFiledButDidntShipReason: buildFiledButDidntShipStopReason,
      buildStaleEpicChildrenReason: buildStaleEpicChildrenStopReason,
    });
    for (const effect of outcome.sideEffects) {
      if (effect.kind === "record-audit-signature") {
        this.lastAuditSignatureByThread.set(threadId, effect.signature);
      } else if (effect.kind === "record-filed-but-didnt-ship-fired") {
        this.filedButDidntShipFiredByThread.add(threadId);
      }
    }
    return outcome.directive ? { ...outcome.directive } : null;
  }

  /**
   * PostToolUse handler: marks the worktree dirty when an edit tool ran,
   * so workspace watchers refresh promptly without waiting for fs events.
   */
  private handlePostToolUseDirty(envelope: HookEnvelope): void {
    const threadId = envelope.threadId;
    if (!threadId || envelope.event !== "PostToolUse") return;
    const payload = (envelope.payload ?? {}) as {
      tool_name?: unknown;
      tool_input?: unknown;
      tool_response?: unknown;
    };
    const toolName = typeof payload.tool_name === "string" ? payload.tool_name : "";
    const toolStatus = derivePostToolStatus(payload.tool_response);
    if (!FILE_EDIT_TOOLS.has(toolName)) return;
    if (toolStatus === "error") return;
    const extractedPath = extractEditedFilePath(payload.tool_input);
    if (!extractedPath) return;
    const thread = this.threadStore.findById(threadId);
    const stream = thread ? this.store.get(thread.stream_id) ?? null : null;
    const normalizedPath = stream
      ? toWorktreeRelativePath(extractedPath, stream.worktree_path)
      : extractedPath;
    if (stream && !shouldAcceptHookFilePath(normalizedPath, stream.worktree_path, this.config.generatedDirs)) {
      return;
    }
    if (thread) this.markDirty(thread.stream_id, normalizedPath);
    // Per-thread wiki note attribution for the rail's Finished list. When
    // the agent writes `.oxplow/notes/<slug>.md` directly via the Write
    // tool (the wiki-capture skill's main path), tag this thread as the
    // author. The watcher will reparse the file shortly after; this
    // records the attribution side row immediately so the rail isn't
    // racing the debounce.
    if (thread && stream) {
      const slug = wikiNoteSlugFromPath(normalizedPath);
      if (slug) {
        this.wikiNoteThreadUpdateStore.recordUpdate(slug, threadId);
      }
    }
  }

  /**
   * Auto-snapshot + effort bookkeeping for a work item status change.
   * `in_progress` opens a new effort with a start snapshot; any transition
   * out of `in_progress` closes it with an end snapshot (subject to a
   * 5-minute minimum gap between snapshots).
   */
  private handleStatusTransition(
    streamId: string,
    threadId: string,
    workItemId: string,
    previous: WorkItem["status"] | undefined,
    next: WorkItem["status"] | undefined,
    touchedFiles?: string[],
  ): void {
    applyStatusTransition(
      {
        effortStore: this.effortStore,
        flushSnapshot: (source, options) =>
          this.safeFlushSnapshot(streamId, source, options?.effortId ?? null),
        getMostRecentSnapshotTimestamp: () =>
          this.snapshotStore.getMostRecentSnapshotTimestampForStream(streamId),
      },
      { threadId, workItemId, previous, next, touchedFiles },
    );
  }

  private safeFlushSnapshot(
    streamId: string,
    source: SnapshotSource,
    effortId: string | null,
  ): string | null {
    try {
      return this.flushSnapshotForStream(streamId, source, effortId);
    } catch (error) {
      this.logger.warn("snapshot flush failed", {
        streamId,
        source,
        error: error instanceof Error ? error.message : String(error),
      });
      return null;
    }
  }

  private markDirty(streamId: string, path: string): void {
    let set = this.dirtyPathsByStream.get(streamId);
    if (!set) {
      set = new Set();
      this.dirtyPathsByStream.set(streamId, set);
    }
    set.add(path);
  }

  /**
   * Capture a snapshot for `streamId` using the accumulated dirty-path set
   * as an optimizer hint. If the snapshot content matches the most recent
   * snapshot (version_hash equal), the existing id is returned with no new
   * row created. Returns null if no stream is found.
   */
  private flushSnapshotForStream(
    streamId: string,
    source: SnapshotSource,
    effortId: string | null = null,
  ): string | null {
    const stream = this.store.get(streamId);
    if (!stream) return null;
    const dirty = this.dirtyPathsByStream.get(streamId);
    const allDirty = dirty ? Array.from(dirty) : [];
    const dirtyPaths = allDirty.filter((relpath) => isInsideWorktree(relpath, stream.worktree_path));
    const ignore = (relpath: string) =>
      shouldIgnoreWorkspaceWatchPath(relpath, this.config.generatedDirs);
    const result = this.snapshotStore.flushSnapshot({
      source,
      streamId,
      worktreePath: stream.worktree_path,
      dirtyPaths: dirty ? dirtyPaths : null,
      ignore,
      effortId,
    });
    if (dirty) dirty.clear();
    if (result.created) {
      this.events.publish({
        type: "file-snapshot.created",
        streamId,
        snapshotId: result.id,
        kind: source,
        effortId: null,
        threadId: null,
      });
    }
    return result.id;
  }

  /**
   * Take a startup snapshot for `streamId` so any changes that happened
   * while the app was down land in history with `source="startup"`. Uses a
   * full worktree walk (no dirty-path optimizer hint — the fs watcher
   * didn't see anything while we were off); `version_hash` dedup means we
   * don't insert a row when nothing actually changed.
   */
  private takeStartupSnapshot(streamId: string): void {
    const stream = this.store.get(streamId);
    if (!stream) return;
    setImmediate(() => {
      if (this.disposed) return;
      try {
        const ignore = (relpath: string) =>
          shouldIgnoreWorkspaceWatchPath(relpath, this.config.generatedDirs);
        const result = this.snapshotStore.flushSnapshot({
          source: "startup",
          streamId,
          worktreePath: stream.worktree_path,
          dirtyPaths: null,
          ignore,
        });
        if (result.created) {
          this.events.publish({
            type: "file-snapshot.created",
            streamId,
            snapshotId: result.id,
            kind: "startup",
            effortId: null,
            threadId: null,
          });
        }
      } catch (error) {
        this.logger.warn("startup snapshot failed", {
          streamId,
          error: error instanceof Error ? error.message : String(error),
        });
      }
    });
  }

  private stampUiWrite(path: string): void {
    this.recentUiWrites.set(path, Date.now());
    if (this.recentUiWrites.size > 500) {
      const cutoff = Date.now() - UI_WRITE_ECHO_WINDOW_MS;
      for (const [key, stamp] of this.recentUiWrites) {
        if (stamp < cutoff) this.recentUiWrites.delete(key);
      }
    }
  }

  listSnapshots(streamId: string, limit?: number): FileSnapshot[] {
    return this.snapshotStore.listSnapshotsForStream(streamId, limit);
  }

  getSnapshotSummary(snapshotId: string, previousSnapshotId?: string | null): SnapshotSummary | null {
    return this.snapshotStore.getSnapshotSummary(snapshotId, previousSnapshotId);
  }

  getSnapshotPairDiff(
    beforeSnapshotId: string | null,
    afterSnapshotId: string,
    path: string,
  ): SnapshotDiffResult {
    return this.snapshotStore.getSnapshotPairDiff(beforeSnapshotId, afterSnapshotId, path);
  }

  /**
   * For each snapshot id in `snapshotIds`, return the list of efforts whose
   * `end_snapshot_id` is that snapshot, annotated with the owning work item's
   * title. Used by the Local History panel to render one row per effort
   * (matching the per-effort write-log attribution model). Snapshots without
   * any ending effort map to an empty array.
   */
  listEffortsEndingAtSnapshots(
    snapshotIds: string[],
  ): Record<string, Array<{ effortId: string; workItemId: string; title: string }>> {
    return this.effortStore.listEffortsEndingAtSnapshots(snapshotIds);
  }

  getEffortFiles(effortId: string): SnapshotSummary | null {
    return computeEffortFiles(this.effortStore, this.snapshotStore, effortId);
  }

  /**
   * Overwrite a file in the stream's worktree with the content it had
   * in `snapshotId`. Uses the existing `writeWorkspaceFile` path so the
   * UI-echo filter and event bus behave identically to a UI edit.
   */
  runSnapshotCleanup(): { snapshotsDeleted: number; blobsDeleted: number } {
    const days = this.config.snapshotRetentionDays;
    try {
      const result = this.snapshotStore.cleanupOldSnapshots(days);
      if (result.snapshotsDeleted > 0 || result.blobsDeleted > 0) {
        this.logger.info("snapshot cleanup", {
          retentionDays: days,
          ...result,
        });
      }
      return result;
    } catch (error) {
      this.logger.warn("snapshot cleanup failed", {
        error: error instanceof Error ? error.message : String(error),
      });
      return { snapshotsDeleted: 0, blobsDeleted: 0 };
    }
  }

  restoreFileFromSnapshot(streamId: string, snapshotId: string, path: string): void {
    const stream = this.store.get(streamId);
    if (!stream) throw new Error(`unknown stream: ${streamId}`);
    const hash = this.snapshotStore.resolvePath(snapshotId, path);
    if (!hash) throw new Error(`snapshot ${snapshotId} has no content for ${path}`);
    const content = this.snapshotStore.readBlob(hash).toString("utf8");
    this.writeWorkspaceFile(streamId, path, content);
  }

  // -------- wiki notes (IPC-exposed) --------

  listWikiNotes(streamId: string): Array<{
    slug: string;
    title: string;
    updated_at: string;
    created_at: string;
    freshness: "fresh" | "stale" | "very-stale";
    head_advanced: boolean;
    changed_refs: string[];
    deleted_refs: string[];
    total_refs: number;
    referenced_files: string[];
  }> {
    const stream = this.resolveStream(streamId);
    const projectDir = stream.worktree_path;
    return this.wikiNoteStore.list().map((n) => {
      const freshness = computeWikiNoteFreshness(
        { capturedHeadSha: n.captured_head_sha, capturedRefs: n.captured_refs },
        readWorktreeHeadSha(projectDir),
        (path) => hashWorkspaceFile(projectDir, path),
      );
      return {
        slug: n.slug,
        title: n.title,
        updated_at: n.updated_at,
        created_at: n.created_at,
        freshness: freshness.status,
        head_advanced: freshness.headAdvanced,
        changed_refs: freshness.changedRefs,
        deleted_refs: freshness.deletedRefs,
        total_refs: freshness.totalRefs,
        referenced_files: n.captured_refs.map((r) => r.path),
      };
    });
  }

  readWikiNoteBody(streamId: string, slug: string): string {
    const stream = this.resolveStream(streamId);
    if (!/^[a-zA-Z0-9_-][a-zA-Z0-9_.-]*$/.test(slug)) throw new Error(`invalid note slug: ${slug}`);
    const path = join(wikiNotesDir(stream.worktree_path), `${slug}.md`);
    if (!existsSync(path)) throw new Error(`note not found: ${slug}`);
    return readFileSync(path, "utf8");
  }

  writeWikiNoteBody(streamId: string, slug: string, body: string): void {
    const stream = this.resolveStream(streamId);
    if (!/^[a-zA-Z0-9_-][a-zA-Z0-9_.-]*$/.test(slug)) throw new Error(`invalid note slug: ${slug}`);
    const dir = join(stream.worktree_path, ".oxplow", "notes");
    mkdirSync(dir, { recursive: true });
    const path = join(dir, `${slug}.md`);
    writeFileSync(path, body, "utf8");
    syncNoteFromDisk(stream.worktree_path, this.wikiNoteStore, slug);
  }

  deleteWikiNote(streamId: string, slug: string): void {
    const stream = this.resolveStream(streamId);
    if (!/^[a-zA-Z0-9_-][a-zA-Z0-9_.-]*$/.test(slug)) throw new Error(`invalid note slug: ${slug}`);
    const path = join(wikiNotesDir(stream.worktree_path), `${slug}.md`);
    try {
      rmSync(path, { force: true });
    } catch {}
    this.wikiNoteStore.deleteBySlug(slug);
    this.wikiNoteThreadUpdateStore.deleteBySlug(slug);
  }

  searchWikiNotes(_streamId: string, query: string, limit?: number): Array<{
    slug: string;
    title: string;
    snippet: string;
    updated_at: string;
  }> {
    return this.wikiNoteStore.searchBodies(query, limit);
  }

  // -------- code quality scans (IPC-exposed) --------

  async runCodeQualityScan(input: {
    streamId: string;
    tool: CodeQualityTool;
    scope: CodeQualityScope;
    baseRef?: string | null;
  }): Promise<CodeQualityScanRow> {
    const stream = this.resolveStream(input.streamId);
    const scanId = this.codeQualityStore.startScan({
      streamId: input.streamId,
      tool: input.tool,
      scope: input.scope,
      baseRef: input.scope === "diff" ? input.baseRef ?? null : null,
    });
    const taskId = this.backgroundTaskStore.start({
      kind: "code-quality",
      label: `${input.tool} scan (${input.scope})`,
    });

    try {
      let files: string[] | undefined;
      if (input.scope === "diff") {
        const baseRef = input.baseRef?.trim() || detectBaseBranch(stream.worktree_path);
        if (!baseRef) {
          this.codeQualityStore.failScan(scanId, "No base ref available for diff scope");
          this.backgroundTaskStore.fail(taskId, "No base ref available for diff scope");
          return this.codeQualityStore.listScans({ streamId: input.streamId }).find((s) => s.id === scanId)!;
        }
        const changes = listBranchChanges(stream.worktree_path, baseRef);
        files = changes.files
          .filter((f) => f.status !== "deleted")
          .map((f) => f.path);
        if (files.length === 0) {
          this.codeQualityStore.completeScan(scanId, []);
          this.backgroundTaskStore.complete(taskId);
          return this.codeQualityStore.listScans({ streamId: input.streamId }).find((s) => s.id === scanId)!;
        }
      }

      const findings = input.tool === "lizard"
        ? await runLizard(stream.worktree_path, { files })
        : await runJscpd(stream.worktree_path, { files });
      this.codeQualityStore.completeScan(scanId, findings);
      this.backgroundTaskStore.complete(taskId);
    } catch (error) {
      const message = error instanceof CodeQualityToolMissingError
        ? `${input.tool} is not installed (install via pip/npm and ensure it's on PATH)`
        : error instanceof Error ? error.message : String(error);
      this.codeQualityStore.failScan(scanId, message);
      this.backgroundTaskStore.fail(taskId, message);
    }

    const row = this.codeQualityStore.listScans({ streamId: input.streamId }).find((s) => s.id === scanId);
    if (!row) throw new Error(`code_quality_scan ${scanId} vanished after run`);
    return row;
  }

  listCodeQualityFindings(input: {
    streamId: string;
    tool?: CodeQualityTool;
    paths?: string[];
  }): CodeQualityFindingRow[] {
    return this.codeQualityStore.listLatestFindings(input);
  }

  listCodeQualityScans(input: { streamId: string; limit?: number }): CodeQualityScanRow[] {
    return this.codeQualityStore.listScans(input);
  }

  // -------- generic usage tracking (IPC-exposed) --------

  recordUsage(input: { kind: string; key: string; event?: string; streamId?: string | null; threadId?: string | null }): void {
    this.usageStore.record(input);
  }

  listRecentUsage(input: { kind: string; streamId?: string | null; threadId?: string | null; limit?: number; since?: string }): Array<{ key: string; last_at: string; count: number }> {
    return this.usageStore.mostRecent(input);
  }

  listFrequentUsage(input: { kind: string; streamId?: string | null; threadId?: string | null; limit?: number; since?: string }): Array<{ key: string; last_at: string; count: number }> {
    return this.usageStore.mostFrequent(input);
  }

  listCurrentlyOpenUsage(input: { kind: string; streamId?: string | null; threadId?: string | null }): string[] {
    return this.usageStore.currentlyOpen(input);
  }

  getWorkItemSummaries(ids: string[]) {
    return this.workItemStore.getSummariesByIds(ids);
  }

  /**
   * Reorder the thread's work-item queue. Sort_indexes are rewritten to
   * match the desired top-to-bottom order. Marker rows (commit/wait
   * points) used to share this sort_index space; they no longer exist
   * so the entry kind is implicit.
   */
  reorderThreadQueue(
    streamId: string,
    threadId: string,
    entries: Array<{ id: string }>,
  ): void {
    this.resolveThread(streamId, threadId);
    this.workItemStore.setItemSortIndexes(threadId, entries.map((entry, index) => ({ id: entry.id, sortIndex: index })));
  }
}

export interface HookHealthReport {
  event: string;
  message: string;
  hint: string;
}

/**
 * Compare the set of registered hook events against the set we've actually
 * received, emitting one warn report per missing event. `SessionStart` gets
 * a dedicated hint (Claude Code silently drops it — see
 * `.context/agent-model.md`); the rest carry a generic "may be fine, may be
 * broken" hint so the user can judge. Pure so the test pins the exact
 * phrasing of both hint kinds.
 */
export function describeHookHealth(
  registered: readonly string[],
  seen: ReadonlySet<string>,
): HookHealthReport[] {
  const out: HookHealthReport[] = [];
  for (const event of registered) {
    if (seen.has(event)) continue;
    if (event === "SessionStart") {
      out.push({
        event,
        message: "registered hook never delivered",
        hint: "Claude Code drops HTTP hooks for SessionStart — the runtime opportunistically learns session_id from later hooks (see decideResumeUpdate).",
      });
    } else {
      out.push({
        event,
        message: "registered hook not observed yet",
        hint: "Expected if the turn didn't exercise this hook (e.g. PreToolUse when no tools ran). If it NEVER fires, the plugin hook registration may be broken.",
      });
    }
  }
  return out;
}

/**
 * Inspect a terminal-pane websocket message string and decide whether
 * it represents a user-issued interrupt (Escape, Ctrl-C). Used by
 * `sendTerminalMessage` to synthesize a meta `Interrupt` hook event so
 * the agent-status reducer can clear "working" — Claude Code doesn't
 * reliably fire Stop on user-driven cancellations.
 *
 * Conservative: only returns true for messages whose decoded input
 * bytes consist *exclusively* of interrupt control bytes. This avoids
 * matching pasted text that happens to contain a stray ESC, or normal
 * keystrokes during a turn.
 */
export function terminalInputIsInterrupt(message: string): boolean {
  let parsed: unknown;
  try { parsed = JSON.parse(message); } catch { return false; }
  if (!parsed || typeof parsed !== "object") return false;
  const obj = parsed as { type?: unknown; bytes?: unknown };
  if (obj.type !== "input" && obj.type !== "input-binary") return false;
  if (typeof obj.bytes !== "string" || obj.bytes.length === 0) return false;
  let decoded: string;
  try { decoded = Buffer.from(obj.bytes, "base64").toString("binary"); }
  catch { return false; }
  if (decoded.length === 0) return false;
  // Accept either a bare ESC (\x1b) or Ctrl-C (\x03), each as the sole
  // payload byte. Multi-byte sequences starting with ESC (e.g. arrow
  // keys: \x1b[A) are NOT interrupts and must not match.
  return decoded === "\x1b" || decoded === "\x03";
}

/**
 * Audit nudge for the Stop-hook in-progress branch. Lists every
 * `in_progress` work item on the thread and asks the agent to reconcile
 * each one. Tasks persist across turn boundaries — without this nudge,
 * stale `in_progress` rows pile up because nothing forces a settle step.
 */
export function buildInProgressAuditStopReason(items: WorkItem[]): string {
  const lines = items.map((item) => `- "${item.title}" (itemId: ${item.id})`);
  return [
    `You have ${items.length} work item${items.length === 1 ? "" : "s"} marked \`in_progress\` on this thread:`,
    ``,
    ...lines,
    ``,
    `Audit each before stopping. For each item:`,
    `- If it's still actively being worked on, leave it \`in_progress\`.`,
    `- If its acceptance criteria are met, call \`mcp__oxplow__complete_task\` (status \`done\`).`,
    `- If you're stuck and need a user decision, \`mcp__oxplow__update_work_item\` with \`status: "blocked"\`.`,
    `- If it's paused but resumable later, \`status: "ready"\`.`,
    `- If it's no longer relevant, \`status: "canceled"\`.`,
    ``,
    `When referring to any of these items to the user in chat, use the quoted title — never the \`wi-…\` id. The id is internal to tool calls.`,
  ].join("\n");
}

export function buildStaleEpicChildrenStopReason(
  pairs: Array<{ epic: WorkItem; staleChildren: WorkItem[] }>,
): string {
  const lines: string[] = [
    `BLOCKED: ${pairs.length === 1 ? "an epic" : `${pairs.length} epics`} on this thread ${pairs.length === 1 ? "is" : "are"} closed (done/blocked) but still ${pairs.length === 1 ? "has" : "have"} non-terminal children. The Plan-pane epic rollup will pull the epic back into To Do, hiding the closed state from the rail counts.`,
    ``,
  ];
  for (const { epic, staleChildren } of pairs) {
    lines.push(`Epic "${epic.title}" (${epic.id}) — status=${epic.status} but has ${staleChildren.length} non-terminal child${staleChildren.length === 1 ? "" : "ren"}:`);
    for (const child of staleChildren) {
      lines.push(`  • ${child.id} (${child.status}) "${child.title}"`);
    }
    lines.push(``);
  }
  lines.push(
    `Fix one of:`,
    `  • If the children's work shipped with the epic, close them via \`mcp__oxplow__transition_work_items\` (target=done or blocked).`,
    `  • If the children still need work, reopen the epic via \`mcp__oxplow__update_work_item\` (status=ready or in_progress).`,
  );
  return lines.join("\n");
}

export function buildFiledButDidntShipStopReason(): string {
  return [
    `ADVISORY: this turn filed at least one new \`ready\` work item but didn't edit any project files, and you have nothing in_progress.`,
    ``,
    `\`status: "ready"\` is for **backlog** — "I noticed this for later". When the user gives a direct instruction ("do this", "yes, proceed", "fix that", "implement X"), the deliverable is the change itself, not the row that tracks it.`,
    ``,
    `If the user told you to do the work this turn:`,
    `  • Reopen the relevant ready row(s): \`mcp__oxplow__update_work_item\` → status=in_progress.`,
    `  • Make the edits. Close back to \`done\` with \`complete_task\`.`,
    ``,
    `If the user only asked you to log/file/remember the items (legitimate backlog capture), reply briefly confirming what you filed and stop — the directive is advisory, not a wall.`,
  ].join("\n");
}

/**
 * UserPromptSubmit-time hint that nudges the agent to capture
 * non-trivial exploratory Q&A turns into the per-project wiki. Fires
 * on both codebase exploration (how/where/explain/trace, architecture
 * walkthroughs) AND general synthesis (why questions, comparisons,
 * tradeoffs, recommendations, rationale) — anywhere a substantive
 * answer would be worth keeping rather than letting it scroll out of
 * chat. Returns `null` when the prompt looks like a fix/feature/yes
 * ack — those don't produce capturable synthesis. Replaces the old
 * Stop-hook wiki-capture directive (which fired post-hoc, after the
 * answer had already been written to chat with no durable home).
 * Path-allowed under the wiki carve-out in `write-guard.ts`, so the
 * hint is valid on read-only threads too.
 */
export function buildWikiCaptureHint(prompt: string): string | null {
  if (typeof prompt !== "string") return null;
  const normalized = prompt.trim();
  if (!normalized) return null;
  // Match anywhere in the prompt — the user often opens with greetings or
  // context before the actual ask. Two regex families: codebase
  // exploration (how/where/architecture/overview) and general synthesis
  // (why/compare/tradeoffs/should-i/rationale/recommend). Either match
  // qualifies the turn for capture.
  const codebaseExploration = /\b(how (?:does|do|is|are|was)|where (?:is|are|does)|explain|trace|describe(?: the)?(?: architecture)?|walk (?:me )?through|give (?:me )?an? (?:overview|summary|tour|architecture)|what (?:is|are) the architecture|architecture of|summari[sz]e (?:the )?(?:code|codebase|module|system)|high(?:[- ]level)? (?:architecture|overview|summary)|overview of (?:the )?(?:code|codebase|module|system))\b/i;
  const generalExploration = /\b(why (?:does|do|is|are|did|didn't|don't|doesn't|isn't|aren't|would|wouldn't|should|shouldn't|can|can't)|what(?:'s| is| are) the (?:difference|tradeoffs?|trade-offs?|pros and cons|advantages|disadvantages|relationship|reason|rationale|impact|implication|consequences?)|compare\b[^.?!]*\b(?:to|and|with|vs|versus)\b|trade-?offs?\b|pros and cons|should (?:i|we|you) (?:use|pick|do|prefer|go with|choose|consider|worry|expect)|best (?:way|practice|approach|strategy|option|choice|pattern)|is (?:it|this|that) (?:better|worse|safer|faster|preferable|safe|correct|right|ok|okay)|advice (?:on|for|about)|recommend(?:ation)?(?: for| on| about)?|rationale (?:for|behind)|reasoning behind)\b/i;
  if (!codebaseExploration.test(normalized) && !generalExploration.test(normalized)) return null;
  return [
    `<wiki-capture-hint>`,
    `This looks like a non-trivial exploratory question — codebase walkthrough, design rationale, comparison, tradeoffs, or general synthesis. Before responding, search existing notes (\`mcp__oxplow__search_notes\` / \`search_note_bodies\` / \`find_notes_for_file\`), then Write \`.oxplow/notes/<slug>.md\` (append-or-create) and call \`mcp__oxplow__resync_note\`. The chat reply should summarize the note, not substitute for it. The wiki is for any durable understanding, not just code questions. The write guard allows this on read-only threads.`,
    `</wiki-capture-hint>`,
  ].join("\n");
}

class RuntimeSocket extends EventEmitter {
  readonly OPEN = 1;
  readonly CLOSED = 3;
  readyState = this.OPEN;

  constructor(private readonly onSend: (message: string) => void) {
    super();
  }

  send(message: string) {
    if (this.readyState !== this.OPEN) return;
    this.onSend(message);
  }

  close() {
    if (this.readyState !== this.OPEN) return;
    this.readyState = this.CLOSED;
    this.emit("close");
  }
}

function buildThreadAgentPrompt(
  stream: Stream,
  thread: Thread,
  agentPromptAppend: string,
  activeThread?: Thread | null,
): string {
  // Keep this preamble SITUATIONAL only — procedural "how to use the work-item
  // tools" policy lives in the `oxplow-runtime` skill (merged: filing +
  // lifecycle + dispatch) so it's only loaded when the agent actually
  // needs it. Every line here is replayed via cache-read on every turn;
  // treat additions as expensive.
  const lines = [
    `SESSION CONTEXT: stream "${stream.title}" (id: ${stream.id}), thread "${thread.title}" (id: ${thread.id}). Pass threadId="${thread.id}" to all oxplow work-item tools.`,
    activeThread && activeThread.id !== thread.id
      ? `ACTIVE (writer) thread: "${activeThread.title}" (id: ${activeThread.id}). Only that thread can commit; your thread is read-only.`
      : `Your thread is the ACTIVE writer — the only thread allowed to commit.`,
    `WORK ITEMS TRACK THE WORK; THEY ARE NOT THE WORK. Filing an item is bookkeeping — the deliverable is the code/docs/config change. Before your first Edit/Write/MultiEdit to project files in a turn, you MUST have an \`in_progress\` work item attributable to this turn — either pre-existing and being continued, or newly created via \`mcp__oxplow__create_work_item\` (status=in_progress) or \`mcp__oxplow__update_work_item\` (→ in_progress). There is no trivial-edit carve-out: typos, single-line CSS tweaks, and one-file fixes all require an item. The Stop hook will block any turn that wrote files without a filing/transition tool call. Pick the shape by structure: \`create_work_item\` with kind \`task\` for one coherent change (even if it spans a few files); \`file_epic_with_children\` when the work has ≥3 sub-steps a reviewer would check off independently. Test: could a child close to \`done\` on its own and have the user inspect just that piece? If no, it's one task. No "auto" placeholder items. **When the work (or each epic child) actually ships, close that row in the same turn** via \`mcp__oxplow__complete_task\` (pass \`touchedFiles\` so Local History can attribute writes) or \`update_work_item\` with \`status: "blocked"\` (need a user decision). Closing an epic does NOT cascade to children — pass them through \`transition_work_items\` in the same turn or the rollup will pull the epic back into To Do. REDO RULE: if the new edits fix/continue something you just closed to \`done\`, REOPEN that item (\`update_work_item\` → in_progress) and re-close it; do NOT file a parallel "Fix …" task. Load the oxplow-runtime skill for tool details.`,
    `READY VS IN_PROGRESS — DON'T CONFLATE THEM. \`status: "ready"\` means "I noticed this for later" (a backlog item). \`status: "in_progress"\` (with edits in the same turn) means "I'm doing this now". When the user gives you a direct instruction ("do this", "yes, proceed", "fix that", "implement X"), default to in_progress + ship in the same turn. Only file as ready when YOU surfaced an idea the user didn't ask for, or when the user explicitly says "log this" / "file as backlog" / "remember to". Filing a ready item in response to "do those" is a misread of the request.`,
    `WHEN YOU ASK THE USER A QUESTION, STOP AND WAIT. If your reply ends with a real clarifying question, an A/B/C choice, or any other ask where the user owns the next move, call \`mcp__oxplow__await_user({ threadId, question })\` and end your turn. Do NOT pick up the next queue item, do NOT call \`read_work_options\`, do NOT dispatch a subagent, do NOT start unrelated work. The Stop hook honours \`await_user\` — it allows-stop and suppresses every directive (commit, audit, ready-work, filing-enforcement) until the user replies. The flag clears automatically on the next user prompt. Don't call \`await_user\` for rhetorical asides or status updates — only for genuine open questions.`,
    `AFTER \`ExitPlanMode\` APPROVAL, KEEP GOING. Plan approval IS the go-ahead — file or transition the implementation work item(s) and start editing in the same turn. Do not pause for a separate "shall I start?" check, and don't \`await_user\` unless something genuinely new is unclear.`,
    `WIKI CAPTURE: any non-trivial exploratory Q&A goes into the wiki — codebase walkthroughs ("how does X work", trace, architecture, overview) AND general synthesis (why questions, comparisons, tradeoffs, design rationale, recommendations, advice). Wiki ≠ codebase-only; it's the durable home for any understanding worth keeping. Write the answer into \`.oxplow/notes/<slug>.md\` via the \`oxplow-wiki-capture\` skill — search existing notes first, append-or-create, then \`mcp__oxplow__resync_note\`. The chat reply summarizes the note, not the other way around. Allowed on read-only threads too (the write guard exempts the notes dir).`,
  ];
  if (thread.status !== "active") {
    lines.push(NON_WRITER_PROMPT_BLOCK);
  }
  const userAppend = agentPromptAppend.trim();
  if (userAppend) {
    lines.push("", "USER CUSTOM PROMPT:", userAppend);
  }
  const streamPrompt = stream.custom_prompt?.trim();
  if (streamPrompt) {
    lines.push("", "# Stream instructions", "", streamPrompt);
  }
  const threadPrompt = thread.custom_prompt?.trim();
  if (threadPrompt) {
    lines.push("", "# Thread instructions", "", threadPrompt);
  }
  return lines.join("\n");
}

/**
 * Pure builder for the <session-context> additionalContext block injected
 * into every UserPromptSubmit. Shape is stable — any renames to the tag
 * name or field order will break agents that learned the previous layout.
 */
export function buildSessionContextBlock(input: {
  stream: { id: string; title: string };
  thread: { id: string; title: string };
  activeThread: { id: string; title: string } | null;
  /**
   * The thread's writer/read-only role at the moment this Claude session
   * was first seen. When set and different from the *current* role, a
   * loud ROLE CHANGE banner is appended before `</session-context>` to
   * supersede the (frozen, cache-read) NON_WRITER_PROMPT_BLOCK in the
   * initial system prompt. Omitting the field is a no-op — older call
   * sites keep the original single-line writer: rendering.
   */
  initialRole?: "writer" | "read-only";
}): string {
  const { stream, thread, activeThread, initialRole } = input;
  const currentRole: "writer" | "read-only" =
    activeThread && activeThread.id !== thread.id ? "read-only" : "writer";
  const lines = [
    `<session-context>`,
    `stream: "${stream.title}" (id: ${stream.id})`,
    `thread:  "${thread.title}" (id: ${thread.id})`,
    activeThread && activeThread.id !== thread.id
      ? `writer: "${activeThread.title}" (id: ${activeThread.id}) — your thread is read-only`
      : `writer: (you) — your thread is the active writer`,
  ];
  if (initialRole && initialRole !== currentRole) {
    if (currentRole === "writer") {
      lines.push(
        "ROLE CHANGE: this thread was read-only when the session started; it is now the active writer. The NON_WRITER block in your initial system prompt is SUPERSEDED — you may now use Write/Edit/Bash to mutate the worktree.",
      );
    } else {
      lines.push(
        "ROLE CHANGE: this thread was the active writer when the session started; it is now read-only. The NON_WRITER block applies now even though it wasn't in your initial system prompt — Write/Edit/Bash mutations to the worktree will be blocked.",
      );
    }
  }
  lines.push(`</session-context>`);
  return lines.join("\n");
}

/**
 * When the most-recently-closed agent-authored item on this thread is
 * `done` and closed within the reminder window (default 15 min), produce
 * a prominent reminder pointing at it. Injected into UserPromptSubmit
 * additionalContext so that when the user's new prompt is likely a
 * correction to that item, the agent sees the reopen path
 * (update_work_item → in_progress) before anything else — instead of
 * filing a duplicate "Fix …" task, silently expanding another item's
 * scope, or forgetting to re-record the effort. Returns empty when no
 * eligible item exists.
 */
export function buildRecentDoneReminder(
  items: WorkItem[],
  now: number,
  windowMs = 15 * 60 * 1000,
): string {
  const cutoff = now - windowMs;
  let candidate: { id: string; title: string; ts: number } | null = null;
  for (const item of items) {
    if (item.status !== "done") continue;
    if (item.author !== "agent") continue;
    const ts = Date.parse(item.updated_at);
    if (!Number.isFinite(ts) || ts < cutoff) continue;
    if (!candidate || ts > candidate.ts) {
      candidate = { id: item.id, title: item.title, ts };
    }
  }
  if (!candidate) return "";
  return [
    "<recent-done-reminder>",
    `You just closed "${candidate.title}" to done on this thread.`,
    "If the user's new prompt is a fix/redo/pushback on THAT item (even indirectly — \"still doesn't work\", \"that's wrong\", \"try again\", \"no\", etc.):",
    `  1. Call update_work_item itemId="${candidate.id}" status=in_progress to reopen it.`,
    "  2. Do the new effort in the same item.",
    "  3. Call complete_task back to done when done (with touchedFiles).",
    "Do NOT file a new \"Fix …\" task for the redo — that fragments history and the Work panel lies about how many concerns were actually raised.",
    "If the new prompt is a GENUINELY separate concern, file a new item as usual and ignore this reminder.",
    "When you mention this item to the user in chat, refer to it by its quoted title — never by its `wi-…` id. The id is internal to tool calls; the user doesn't see or know it.",
    "</recent-done-reminder>",
  ].join("\n");
}

/**
 * When a UserPromptSubmit arrives and at least one `in_progress` item
 * already exists on this thread (i.e. it was started under a prior
 * prompt and is still open), produce a reminder pointing at it. The
 * agent then either files a fresh row (new concern) or explicitly
 * reopens the existing one (fix/redo) — instead of silently bundling
 * the new ask into whatever happened to be open. Returns empty when
 * the thread has no `in_progress` items.
 */
export function buildPriorPromptInProgressReminder(
  items: WorkItem[],
  now: number,
): string {
  // Pick the most-recently-touched in_progress item — that's the one
  // the agent's most likely to bundle a new ask into.
  let candidate: { id: string; title: string; ts: number } | null = null;
  for (const item of items) {
    if (item.status !== "in_progress") continue;
    const ts = Date.parse(item.updated_at);
    // Items in_progress in the future (clock skew) shouldn't suppress
    // the reminder — fall through with ts = -Infinity instead.
    const safeTs = Number.isFinite(ts) && ts <= now ? ts : -Infinity;
    if (!candidate || safeTs > candidate.ts) {
      candidate = { id: item.id, title: item.title, ts: safeTs };
    }
  }
  if (!candidate) return "";
  return [
    "<prior-prompt-in-progress-reminder>",
    `A new user prompt arrived while "${candidate.title}" (id ${candidate.id}) is still in_progress on this thread from a prior prompt.`,
    "Default to treating this as a NEW ask:",
    "  • Separate concern → `mcp__oxplow__create_work_item` with status=in_progress, then ship and `complete_task`.",
    `  • Fix/redo of "${candidate.title}" → \`mcp__oxplow__update_work_item\` itemId="${candidate.id}" to keep working under that item; do NOT bundle silently.`,
    "Bundling a new ask into an unrelated open item makes the Work panel under-report the concerns the user actually raised.",
    "When you mention this item to the user in chat, refer to it by its quoted title — never by its `wi-…` id.",
    "</prior-prompt-in-progress-reminder>",
  ].join("\n");
}

/**
 * When a UserPromptSubmit arrives and exactly one `ready` item on the
 * thread plausibly matches the new prompt by content (≥2 shared
 * significant tokens), produce a reminder pointing at it. Catches the
 * failure mode where the agent files a fresh task that duplicates a
 * ready row already on the board, instead of flipping the existing
 * row to in_progress. Returns "" when there's no match, when the
 * match is ambiguous (multiple ready items score similarly), or when
 * the prompt itself is too short to extract meaningful tokens.
 *
 * Heuristic — intentionally cheap and conservative:
 *   1. Tokenize prompt and item (title + description) into lowercase
 *      alphanumeric runs of length ≥ 4 that aren't in `STOP_TOKENS`.
 *   2. Score each ready item = |prompt-tokens ∩ item-tokens|.
 *   3. Emit only when the top-scoring item has ≥ 2 shared tokens AND
 *      no other ready item is within 1 of its score. Ambiguous → skip
 *      (the spec is "clear single match"); the agent will file a new
 *      row, which is the safer default.
 */
export function buildReadyMatchReminder(
  items: WorkItem[],
  promptText: string,
): string {
  const promptTokens = tokenizeForReadyMatch(promptText);
  if (promptTokens.size < 2) return "";
  const scored: Array<{ id: string; title: string; score: number }> = [];
  for (const item of items) {
    if (item.status !== "ready") continue;
    const itemTokens = tokenizeForReadyMatch(`${item.title} ${item.description ?? ""}`);
    let score = 0;
    for (const tok of promptTokens) if (itemTokens.has(tok)) score++;
    if (score >= 2) scored.push({ id: item.id, title: item.title, score });
  }
  if (scored.length === 0) return "";
  scored.sort((a, b) => b.score - a.score);
  const top = scored[0]!;
  const runnerUp = scored[1];
  if (runnerUp && top.score - runnerUp.score < 2) return "";
  return [
    "<ready-item-match-reminder>",
    `The user's new prompt looks like it could be the existing ready item "${top.title}" (id ${top.id}) on this thread.`,
    "If it is — flip that row to in_progress instead of filing a duplicate:",
    `  • \`mcp__oxplow__update_work_item\` itemId="${top.id}" status=in_progress, then ship and \`complete_task\`.`,
    "If it's a different concern, file a new row as usual and ignore this reminder.",
    "Filing a fresh task that duplicates an existing ready row fragments the backlog.",
    "When you mention this item to the user in chat, refer to it by its quoted title — never by its `wi-…` id.",
    "</ready-item-match-reminder>",
  ].join("\n");
}

const READY_MATCH_STOP_TOKENS = new Set([
  "this", "that", "with", "from", "have", "your", "what", "when", "where",
  "which", "their", "they", "them", "then", "than", "into", "onto", "some",
  "make", "made", "thing", "things", "stuff", "should", "would", "could",
  "will", "just", "like", "also", "even", "very", "much", "many", "more",
  "most", "such", "still", "after", "before", "while", "about", "over",
  "under", "again", "back", "here", "there", "want", "need", "needs",
  "doesn", "didn", "wasn", "isn", "aren", "won", "don", "the", "and",
  "for", "but", "you", "your", "our", "all", "any", "out", "off", "own",
  "ask", "user", "users", "users", "item", "items", "work", "thread",
  "threads", "prompt", "prompts", "turn", "turns", "agent", "agents",
  "code", "page", "pages", "file", "files", "fix", "make", "add", "do",
  "does", "did", "be", "is", "are", "was", "were", "an", "as", "in",
  "on", "of", "to", "if", "or", "by", "we", "us",
]);

function tokenizeForReadyMatch(text: string): Set<string> {
  const out = new Set<string>();
  if (typeof text !== "string") return out;
  const matches = text.toLowerCase().match(/[a-z0-9]{4,}/g) ?? [];
  for (const tok of matches) {
    if (READY_MATCH_STOP_TOKENS.has(tok)) continue;
    out.add(tok);
  }
  return out;
}

export function buildThreadMcpConfig(mcp: McpServerHandle | null): string {
  if (!mcp) {
    throw new Error("mcp server not started");
  }
  return JSON.stringify({
    mcpServers: {
      oxplow: {
        type: "http",
        url: mcp.httpUrl,
        headers: {
          Authorization: `Bearer ${mcp.authToken}`,
        },
      },
    },
  });
}

function sanitizeProjectBase(projectDir: string): string {
  return resolve(projectDir).split("/").pop()!.replace(/[^a-zA-Z0-9_-]/g, "_");
}

function cleanupSessions(streams: Stream[]) {
  const sessions = new Set(streams.map((stream) => stream.panes.working.split(":")[0]));
  for (const session of sessions) {
    killSession(session);
  }
}

/**
 * New worktrees live as siblings of the main repo: `<parent>/<repo>-<branch>`.
 * Keeps them out of the main repo tree (so they don't get picked up by
 * ignored-file scans, builds, or `git` invocations inside projectDir) while
 * staying visible to the user in a familiar location.
 */
function streamWorktreePath(projectDir: string, branch: string): string {
  const parent = dirname(projectDir);
  const repoName = basename(projectDir);
  return join(parent, `${repoName}-${sanitizeBranch(branch)}`);
}

function sanitizeBranch(branch: string): string {
  return branch.replace(/[^a-zA-Z0-9._-]+/g, "-");
}

function localBranchName(remoteName: string): string {
  const slash = remoteName.indexOf("/");
  return slash >= 0 ? remoteName.slice(slash + 1) : remoteName;
}

function parseLogLevel(value: unknown): LogLevel {
  return value === "debug" || value === "info" || value === "warn" || value === "error" ? value : "info";
}

const UI_WRITE_ECHO_WINDOW_MS = 1000;
const FILE_EDIT_TOOLS = new Set(["Write", "Edit", "MultiEdit", "NotebookEdit"]);

/**
 * If a worktree-relative path points at a wiki note file
 * (`.oxplow/notes/<slug>.md`), return the slug; otherwise null. Used by
 * the PostToolUse handler to attribute Write-driven note edits to the
 * authoring thread without going through the file watcher.
 */
export function wikiNoteSlugFromPath(relPath: string): string | null {
  const normalized = relPath.replace(/^\.\//, "").replace(/\\/g, "/");
  const m = /^\.oxplow\/notes\/([^/]+)\.md$/.exec(normalized);
  return m ? m[1]! : null;
}

function extractEditedFilePath(input: unknown): string | null {
  if (!input || typeof input !== "object") return null;
  const obj = input as Record<string, unknown>;
  if (typeof obj.file_path === "string") return obj.file_path;
  if (typeof obj.notebook_path === "string") return obj.notebook_path;
  if (typeof obj.path === "string") return obj.path;
  return null;
}

export { isInsideWorktree } from "./runtime-paths.js";

/**
 * Decide whether a hook-reported path should produce a file_change row +
 * dirty-path marker. Extracted for testability; mirrors the branch in
 * ElectronRuntime.persistFileChange.
 */
export function shouldAcceptHookFilePath(
  path: string,
  worktreeRoot: string,
  extraIgnoreDirs: string[] = [],
): boolean {
  if (!isInsideWorktree(path, worktreeRoot)) return false;
  const rel = toWorktreeRelativePath(path, worktreeRoot);
  if (shouldIgnoreWorkspaceWatchPath(rel, extraIgnoreDirs)) return false;
  return true;
}

function toWorktreeRelativePath(absOrRel: string, worktreePath: string): string {
  const normalizedRoot = resolve(worktreePath);
  const candidate = resolve(normalizedRoot, absOrRel);
  if (candidate === normalizedRoot) return "";
  if (candidate.startsWith(normalizedRoot + sep)) {
    return candidate.slice(normalizedRoot.length + 1);
  }
  return absOrRel;
}

/**
 * Effort bookkeeping for a work-item status change. Exported so tests can
 * exercise the logic without constructing a full `ElectronRuntime`.
 *
 * - `→ in_progress` (from anything else): flush a `task-start` snapshot,
 *   open an effort linked to it, and attach any currently-open turn.
 * - `in_progress → anything else`: flush a `task-end` snapshot and close
 *   the effort.
 * - Same-status "transitions" (no-op): nothing happens.
 */
export interface StatusTransitionDeps {
  effortStore: WorkItemEffortStore;
  /**
   * Capture a snapshot tagged with `source`. When `options.effortId` is
   * provided the row is attributed to that effort. The runtime
   * implementation translates this into a `flushSnapshotForStream` call;
   * tests pass a synchronous fake.
   */
  flushSnapshot: (
    source: SnapshotSource,
    options?: { effortId?: string | null },
  ) => string | null;
  /**
   * ISO timestamp of the most recent snapshot in the stream this work
   * item belongs to, or null if there are none. Consulted on the
   * effort-end path to enforce the 5-minute minimum gap rule —
   * `task-end` snapshots are skipped when this is younger than
   * `END_SNAPSHOT_MIN_GAP_MS`. Effort starts always fire regardless.
   * Optional: when omitted, the gap rule is not enforced (the end
   * snapshot is always flushed). Tests that don't care about snapshots
   * pass a no-op `flushSnapshot` and skip this dep.
   */
  getMostRecentSnapshotTimestamp?: () => string | null;
}

/**
 * Minimum elapsed time between the most recent snapshot for a stream
 * and a fresh `task-end` snapshot. Below this gap the effort-close
 * path skips the flush and leaves `effort.end_snapshot_id` null —
 * computeEffortFiles already tolerates a null end (returns null) and
 * the next status transition or startup snapshot will close the
 * history range. Effort START snapshots are exempt: they anchor the
 * diff range and need a stable id even if the stream just snapshotted.
 */
export const END_SNAPSHOT_MIN_GAP_MS = 5 * 60 * 1000;

/**
 * Server-side cap on declared touched-file lists. Payloads larger than
 * this are dropped entirely, which matches the "assume all files"
 * fallback in `computeEffortFiles` (empty log → raw pair-diff). Keeps
 * an agent with a degenerate list from flooding the table.
 */
export const TOUCHED_FILES_CAP = 100;

export function applyStatusTransition(
  deps: StatusTransitionDeps,
  params: {
    threadId: string;
    workItemId: string;
    previous: WorkItem["status"] | undefined;
    next: WorkItem["status"] | undefined;
    touchedFiles?: string[];
  },
): void {
  const { previous, next, workItemId, touchedFiles } = params;
  if (!next) return;
  if (next === "in_progress" && previous !== "in_progress") {
    // Open the effort first with a null start snapshot so we have an id
    // to attribute the start snapshot row to; then re-attach the id.
    // Effort starts always fire regardless of recent-snapshot gap — the
    // start row anchors the diff range.
    const opened = deps.effortStore.openEffort({ workItemId, startSnapshotId: null });
    const startSnapshotId = deps.flushSnapshot("task-start", { effortId: opened.id });
    if (startSnapshotId && !opened.start_snapshot_id) {
      // Round-trip through openEffort to write the resolved id back onto
      // the row. openEffort is idempotent and patches start_snapshot_id
      // when the existing row's value is null.
      deps.effortStore.openEffort({ workItemId, startSnapshotId });
    }
  } else if (previous === "in_progress" && next !== "in_progress") {
    // Capture the effort id *before* closing — closeEffort clears the
    // "open effort" marker. We need the id to attach the touched-files
    // payload to the row just closed.
    const openEffort = deps.effortStore.getOpenEffort(workItemId);
    // Any move out of in_progress (done / blocked / ready /
    // canceled / archived) triggers a possible task-end snapshot — the
    // work just paused, so capture the worktree state. The 5-minute gap
    // rule suppresses the flush when the stream's most recent snapshot
    // is younger than the threshold so back-to-back transitions don't
    // pile up near-identical rows. Leaves effort.end_snapshot_id null
    // in the skip case; computeEffortFiles already tolerates a null end.
    const lastSnapTs = deps.getMostRecentSnapshotTimestamp?.() ?? null;
    const skipEnd = lastSnapTs !== null && shouldSkipEndSnapshot(lastSnapTs, Date.now());
    const endSnapshotId = skipEnd
      ? null
      : deps.flushSnapshot("task-end", { effortId: openEffort?.id ?? null });
    deps.effortStore.closeEffort({ workItemId, endSnapshotId });
    if (openEffort && (next === "done" || next === "blocked") && Array.isArray(touchedFiles) && touchedFiles.length > 0) {
      // Dedup, then enforce the cap. Oversized payloads drop ALL rows
      // so computeEffortFiles falls back to raw pair-diff ("assume all").
      const deduped = Array.from(new Set(touchedFiles.filter((p) => typeof p === "string" && p.length > 0)));
      if (deduped.length > 0 && deduped.length <= TOUCHED_FILES_CAP) {
        for (const path of deduped) {
          deps.effortStore.recordEffortFile(openEffort.id, path);
        }
      }
    }
  } else if (previous && next && previous !== next) {
    // Any other status transition (e.g. ready ↔ blocked, done → ready,
    // etc.) where neither side is `in_progress`. No effort opens
    // or closes, but capture the worktree state with a `task-event`
    // snapshot so the user sees these moments in Local History. Same
    // 5-minute gap rule as task-end so back-to-back changes don't pile
    // up near-identical rows.
    const lastSnapTs = deps.getMostRecentSnapshotTimestamp?.() ?? null;
    const skip = lastSnapTs !== null && shouldSkipEndSnapshot(lastSnapTs, Date.now());
    if (!skip) deps.flushSnapshot("task-event", { effortId: null });
  }
}

/**
 * Per-effort file list. Computes the pair-diff over
 * (start_snapshot_id, end_snapshot_id); when 2+ efforts end at the same
 * snapshot AND this effort has ≥1 row in `work_item_effort_file`, the
 * result is filtered to those paths so parallel subagents each see only
 * their own writes. If this effort has zero rows (agent skipped
 * `touchedFiles` on the done transition, or list exceeded the
 * server cap), we fall back to the raw pair-diff — the "assume all"
 * behaviour. The 1-effort case also returns the raw pair-diff. Returns
 * null when the effort is unknown or still open (no end snapshot).
 */
/**
 * Pure helper: return true when `nowMs - lastIso` is below
 * `END_SNAPSHOT_MIN_GAP_MS`. Exported for testability — the real
 * runtime path passes `Date.now()` as `nowMs`.
 */
export function shouldSkipEndSnapshot(lastIso: string, nowMs: number): boolean {
  const lastMs = Date.parse(lastIso);
  if (!Number.isFinite(lastMs)) return false;
  return nowMs - lastMs < END_SNAPSHOT_MIN_GAP_MS;
}

/**
 * Repo-relative paths touched by `effort`. Resolves the same way as
 * `computeEffortFiles` (snapshot pair-diff filtered by per-effort write
 * log when present) but returns just the path list — useful for
 * commit-message preludes, blame overlays, and any consumer that
 * doesn't care about A/M/D classification or per-file SnapshotEntry
 * data. Returns an empty array when the effort is unknown, still open
 * (no end snapshot), or had no recorded changes. Anchored on
 * `effort.start_snapshot_id`/`end_snapshot_id` — the post-turn-removal
 * replacement for the deleted `computeTurnFilePaths`.
 */
export function computeEffortFilePaths(
  effortStore: WorkItemEffortStore,
  snapshotStore: SnapshotStore,
  effortId: string,
): string[] {
  const summary = computeEffortFiles(effortStore, snapshotStore, effortId);
  if (!summary) return [];
  return Object.keys(summary.files).sort();
}

export function computeEffortFiles(
  effortStore: WorkItemEffortStore,
  snapshotStore: SnapshotStore,
  effortId: string,
): SnapshotSummary | null {
  const effort = effortStore.getById(effortId);
  if (!effort) return null;
  const endId = effort.end_snapshot_id;
  if (!endId) return null;
  const summary = snapshotStore.getSnapshotSummary(endId, effort.start_snapshot_id);
  if (!summary) return null;
  const siblings = effortStore
    .listEffortsForSnapshot(endId)
    .filter((row) => row.end_snapshot_id === endId);
  if (siblings.length < 2) return summary;
  const recorded = effortStore.listEffortFiles(effortId);
  // No declared touched-files for this effort: fall back to the raw
  // pair-diff ("assume all"). Better to over-report than to silently
  // show nothing.
  if (recorded.length === 0) return summary;
  const allowed = new Set(recorded);
  const filteredFiles: typeof summary.files = {};
  let created = 0;
  let updated = 0;
  let deleted = 0;
  for (const [path, row] of Object.entries(summary.files)) {
    if (!allowed.has(path)) continue;
    filteredFiles[path] = row;
    if (row.kind === "created") created++;
    else if (row.kind === "updated") updated++;
    else deleted++;
  }
  return {
    snapshot: summary.snapshot,
    previousSnapshotId: summary.previousSnapshotId,
    files: filteredFiles,
    counts: { created, updated, deleted },
  };
}

/** Tool names that always count as write-intent. */
const ALWAYS_WRITE_INTENT_TOOLS: ReadonlySet<string> = new Set([
  "Write", "Edit", "MultiEdit", "NotebookEdit",
]);

/** Tool names that count as "turn produced real work" even if nothing
 *  was written to disk: filing / transition / dispatch calls. Combined
 *  with the write-intent check (Edits, Writes, non-readonly Bash) in
 *  `isActivityTool` to form the signal the Stop-hook uses to decide
 *  whether to fire the ready-work directive. See wi-0f1492f5e60e. */
const FILING_AND_DISPATCH_ACTIVITY_TOOLS: ReadonlySet<string> = new Set([
  "Task",
  "mcp__oxplow__create_work_item",
  "mcp__oxplow__file_epic_with_children",
  "mcp__oxplow__complete_task",
  "mcp__oxplow__update_work_item",
  "mcp__oxplow__transition_work_items",
  "mcp__oxplow__add_work_note",
  "mcp__oxplow__dispatch_work_item",
]);

/**
 * Did a tool call count as mutation / filing / dispatch activity for
 * the current turn? Used by the Stop-hook ready-work suppression (a
 * turn with zero activity is treated as pure Q&A and suppresses the
 * directive; any activity re-arms it). The write-intent check subsumes
 * Edits, Writes, and non-readonly Bash; the filing/dispatch set adds
 * the MCP tools that represent real work without touching the worktree.
 */
export function isActivityTool(toolName: string, toolInput: unknown): boolean {
  if (FILING_AND_DISPATCH_ACTIVITY_TOOLS.has(toolName)) return true;
  if (isWriteIntentTool(toolName, toolInput)) return true;
  return false;
}

/** Leading tokens that mark a Bash command as read-only. Matched case-sensitively
 *  against the first non-whitespace word of the command (`git log`, `git diff`,
 *  etc. are handled with a second token). */
const READONLY_BASH_LEADING: ReadonlySet<string> = new Set([
  "ls", "cat", "grep", "find", "head", "tail", "wc", "rg", "pwd", "echo", "which",
]);
const READONLY_BASH_TWO_WORD: ReadonlySet<string> = new Set([
  "git log", "git diff", "git status", "git show", "git blame",
  // `git commit` / `git push` modify .git but not project files, and the
  // work being committed is already tracked by the work items that
  // landed earlier — counting them as write-intent makes the filing-
  // enforcement Stop branch fire on every "commit my changes" turn,
  // forcing the agent to file a placeholder "Commit XYZ" item just to
  // attribute the Bash invocation. They still count as activity (a
  // turn with a commit isn't a Q&A turn) because `isReadIntentTool`'s
  // "Bash that isn't write-intent counts as a read" rule pulls them
  // into the read column.
  "git commit", "git push",
  "bun test", "bunx tsc",
]);

/**
 * Decide whether a tool call should trigger the auto-file path. The
 * whitelist in ALWAYS_WRITE_INTENT_TOOLS is hard; Bash is write-intent
 * unless its command starts with an obvious read-only verb. False
 * positives are cheap (an extra auto-filed item) — we err toward
 * write-intent when unsure so we don't miss genuine edits.
 */
/** Tools that count as "read activity" for the wiki-capture exploration
 *  heuristic. Read/Grep/Glob always count; Bash counts only when its
 *  command starts with a known read-only verb (same allowlist used by
 *  `isWriteIntentTool` for the inverse decision). MCP read tools that
 *  aren't filing/dispatch are intentionally excluded — those don't
 *  represent code exploration. */
const ALWAYS_READ_INTENT_TOOLS: ReadonlySet<string> = new Set([
  "Read", "Grep", "Glob",
]);

export function isReadIntentTool(toolName: string, toolInput: unknown): boolean {
  if (ALWAYS_READ_INTENT_TOOLS.has(toolName)) return true;
  if (toolName !== "Bash") return false;
  // Read-only Bash counts as a read; write-intent Bash doesn't.
  return !isWriteIntentTool(toolName, toolInput);
}

export function isWriteIntentTool(toolName: string, toolInput: unknown): boolean {
  if (ALWAYS_WRITE_INTENT_TOOLS.has(toolName)) return true;
  if (toolName !== "Bash") return false;
  const cmd = (toolInput && typeof toolInput === "object"
    && typeof (toolInput as { command?: unknown }).command === "string")
    ? (toolInput as { command: string }).command.trim()
    : "";
  if (!cmd) return true; // empty-shaped bash call defaults to write-intent
  const firstSpace = cmd.indexOf(" ");
  const firstWord = firstSpace === -1 ? cmd : cmd.slice(0, firstSpace);
  if (READONLY_BASH_LEADING.has(firstWord)) return false;
  const twoWordKey = firstSpace === -1 ? firstWord : cmd.slice(0, cmd.indexOf(" ", firstSpace + 1) === -1 ? cmd.length : cmd.indexOf(" ", firstSpace + 1));
  if (READONLY_BASH_TWO_WORD.has(twoWordKey)) return false;
  return true;
}

/**
 * Rough byte-size of a PostToolUse tool_response payload for the
 * running-turn cost estimator. String-serializes once per call; within
 * 20% of token weight at the multi-M scale we actually care about (the
 * work item explicitly scopes the estimate that loosely). Returns 0 on
 * anything unrecognisable so the hook never throws on weird payloads.
 */
export function estimateToolResponseBytes(resp: unknown): number {
  if (resp == null) return 0;
  if (typeof resp === "string") return resp.length;
  try {
    return JSON.stringify(resp).length;
  } catch {
    return 0;
  }
}

/**
 * Parse a TodoWrite `tool_input` payload into the final todo list shape
 * the task-list bridge needs. TodoWrite is declarative: the full list is
 * sent on each call with per-item `{ content, status }` (statuses are
 * `pending`, `in_progress`, `completed`). Returns null for malformed
 * payloads.
 */
export function extractTodoList(input: unknown): Array<{ content: string; status: string }> | null {
  if (!input || typeof input !== "object") return null;
  const todos = (input as { todos?: unknown }).todos;
  if (!Array.isArray(todos)) return null;
  const out: Array<{ content: string; status: string }> = [];
  for (const entry of todos) {
    if (!entry || typeof entry !== "object") continue;
    const e = entry as Record<string, unknown>;
    const content = typeof e.content === "string" ? e.content : null;
    const status = typeof e.status === "string" ? e.status : "pending";
    if (content) out.push({ content, status });
  }
  return out;
}

function derivePostToolStatus(resp: unknown): "ok" | "error" {
  if (!resp || typeof resp !== "object") return "ok";
  const obj = resp as Record<string, unknown>;
  if (obj.error != null && obj.error !== "") return "error";
  if (obj.is_error === true) return "error";
  return "ok";
}

/**
 * Walk up from `process.cwd()` looking for a directory that contains
 * both `src/mcp` and `src/persistence` — the two trees the dev-reload
 * watcher observes. Returns null when not found (production Electron
 * installs, tests with a fresh mkdtemp project dir, etc.) — callers
 * are expected to no-op cleanly in that case. Bounded to 8 levels so
 * we never scan past a sensible repo root.
 */
function findSourceRoot(): string | null {
  let dir = process.cwd();
  for (let i = 0; i < 8; i++) {
    if (
      existsSync(join(dir, "src", "mcp")) &&
      existsSync(join(dir, "src", "persistence"))
    ) {
      return dir;
    }
    const parent = dirname(dir);
    if (parent === dir) return null;
    dir = parent;
  }
  return null;
}
