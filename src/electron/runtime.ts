import { EventEmitter } from "node:events";
import { existsSync, mkdirSync, readFileSync, rmSync, watch, writeFileSync, type FSWatcher } from "node:fs";
import { basename, dirname, join, resolve, sep } from "node:path";
import { randomUUID } from "node:crypto";
import { buildAgentCommandForSession } from "../agent/agent-command.js";
import { buildWriteGuardResponse, NON_WRITER_PROMPT_BLOCK } from "./write-guard.js";
import { decideStopDirective, type ThreadSnapshot } from "./stop-hook-pipeline.js";
import { ThreadQueueOrchestrator } from "./thread-queue-orchestrator.js";
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
  gitPull,
  gitCommitAll,
  isWorktreeClean,
  listFileCommits,
  gitBlame,
  listAllRefs,
  listGitRefsGrouped,
  renameBranch,
  deleteBranch,
  gitMerge,
  gitRebase,
  listExistingWorktrees,
  type GitWorktreeEntry,
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
import { CommitPointStore, type CommitPoint } from "../persistence/commit-point-store.js";
import { WikiNoteStore, computeFreshness as computeWikiNoteFreshness, type WikiNote } from "../persistence/wiki-note-store.js";
import { NotesWatcher, hashWorkspaceFile, notesDir as wikiNotesDir, syncNoteFromDisk } from "../git/notes-watch.js";
import { buildWikiNoteMcpTools } from "../mcp/wiki-note-mcp-tools.js";
import { WaitPointStore, type WaitPoint } from "../persistence/wait-point-store.js";
import { WorkItemEffortStore } from "../persistence/work-item-effort-store.js";
import { FollowupStore, type Followup } from "./followup-store.js";
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
  readonly commitPointStore: CommitPointStore;
  readonly wikiNoteStore: WikiNoteStore;
  private notesWatcher: NotesWatcher | null = null;
  readonly waitPointStore: WaitPointStore;
  readonly threadQueue: ThreadQueueOrchestrator;
  readonly effortStore: WorkItemEffortStore;
  readonly snapshotStore: SnapshotStore;
  /** Transient in-memory follow-up store. Backs the agent-only
   *  `add_followup` / `remove_followup` MCP tools and surfaces entries
   *  in the To Do section of the Work panel. No DB row, lost on
   *  runtime restart — see followup-store.ts. */
  readonly followupStore: FollowupStore;
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
  /** Per-thread record of the most recent `read_work_options` call: the set
   *  of ready-item ids the agent saw. Consumed by the next Stop-hook
   *  decision (then cleared) so the ready-work directive doesn't echo the
   *  list the agent already has. Populated through the
   *  markReadWorkOptions callback wired into the MCP tool surface. */
  private readonly lastReadWorkOptionsByThread = new Map<string, string[]>();
  /** Per-thread count of `Task` (subagent) tool calls that are
   *  PreToolUse-started but haven't yet seen a PostToolUse. Used by the
   *  Stop-hook pipeline to suppress the in-progress audit and ready-work
   *  directives while a subagent owns the in_progress item — re-firing
   *  those nudges produces a visual loop where the parent acks each
   *  Stop while still waiting on the subagent. See wi-593a50b62e22. */
  private readonly pendingSubagentsByThread = new Map<string, number>();
  /** Per-thread fingerprint of the in_progress set the runtime last
   *  emitted an audit directive for. The Stop pipeline compares the
   *  current snapshot against this and suppresses a duplicate audit
   *  nudge when nothing changed (no item's `updated_at` ticked, no note
   *  landed, set membership is identical). Cleared lazily by the next
   *  audit fire that records a fresh signature. See wi-c468e8fc093d. */
  private readonly lastAuditSignatureByThread = new Map<string, string>();
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
    this.commitPointStore = new CommitPointStore(projectDir, logger.child({ subsystem: "commit-points" }));
    this.wikiNoteStore = new WikiNoteStore(projectDir, logger.child({ subsystem: "wiki-notes" }));
    this.notesWatcher = new NotesWatcher(
      projectDir,
      this.wikiNoteStore,
      {},
      logger.child({ subsystem: "notes-watch" }),
    );
    this.notesWatcher.start();
    this.waitPointStore = new WaitPointStore(projectDir, logger.child({ subsystem: "wait-points" }));
    this.threadQueue = new ThreadQueueOrchestrator(
      this.store,
      this.threadStore,
      this.workItemStore,
      this.commitPointStore,
      this.waitPointStore,
      logger.child({ subsystem: "thread-queue" }),
    );
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
    this.lspManager = new LspSessionManager(logger.child({ subsystem: "lsp" }));
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
    this.waitPointStore.subscribe((change) => {
      const thread = this.threadStore.findById(change.threadId);
      this.events.publish({
        type: "wait-point.changed",
        streamId: thread?.stream_id ?? null,
        threadId: change.threadId,
        id: change.id,
        kind: change.kind,
      });
    });
    this.commitPointStore.subscribe((change) => {
      const thread = this.threadStore.findById(change.threadId);
      this.events.publish({
        type: "commit-point.changed",
        streamId: thread?.stream_id ?? null,
        threadId: change.threadId,
        id: change.id,
        kind: change.kind,
      });
    });
    this.followupStore.subscribe((change) => {
      this.events.publish({
        type: "followup.changed",
        threadId: change.threadId,
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
          commitPointStore: this.commitPointStore,
          executeCommit: (cpId, message) => this.threadQueue.executeCommit(cpId, message),
          executeAutoCommit: (threadId, message) => this.executeAutoCommitForThread(threadId, message),
          waitPointStore: this.waitPointStore,
          effortStore: this.effortStore,
          markReadWorkOptions: (threadId, readyIds) => this.markReadWorkOptions(threadId, readyIds),
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
            commitPointStore: this.commitPointStore,
            executeCommit: (cpId, message) => this.threadQueue.executeCommit(cpId, message),
            executeAutoCommit: (threadId, message) => this.executeAutoCommitForThread(threadId, message),
            waitPointStore: this.waitPointStore,
            effortStore: this.effortStore,
            markReadWorkOptions: (threadId, readyIds) => this.markReadWorkOptions(threadId, readyIds),
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
    return this.agentStatusByThread.get(threadId) ?? "idle";
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

  listGitRefs(): GroupedGitRefs {
    return listGitRefsGrouped(this.projectDir);
  }

  renameGitBranch(from: string, to: string): GitOpResult {
    return renameBranch(this.projectDir, from, to);
  }

  deleteGitBranch(branch: string, options?: { force?: boolean }): GitOpResult {
    return deleteBranch(this.projectDir, branch, options?.force);
  }

  gitMergeInto(streamId: string, other: string): GitOpResult {
    const stream = this.resolveStream(streamId);
    return gitMerge(stream.worktree_path, other);
  }

  gitRebaseOnto(streamId: string, onto: string): GitOpResult {
    const stream = this.resolveStream(streamId);
    return gitRebase(stream.worktree_path, onto);
  }

  /**
   * Enumerate existing git worktrees that aren't already tracked as oxplow
   * streams. Powers the new-stream "adopt existing worktree" flow. The main
   * worktree (the project itself) is excluded since it's the primary stream.
   */
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

  getGitLog(streamId: string, options?: { limit?: number }) {
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

  gitPush(streamId: string, options?: Parameters<typeof gitPush>[1]): GitOpResult {
    const stream = this.resolveStream(streamId);
    return gitPush(stream.worktree_path, options);
  }

  gitPull(streamId: string, options?: Parameters<typeof gitPull>[1]): GitOpResult {
    const stream = this.resolveStream(streamId);
    return gitPull(stream.worktree_path, options);
  }

  gitCommitAll(streamId: string, message: string, options?: { includeUntracked?: boolean }): GitOpResult & { sha?: string } {
    const stream = this.resolveStream(streamId);
    return gitCommitAll(stream.worktree_path, message, options);
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

  completeThread(streamId: string, threadId: string): ThreadState {
    this.resolveStream(streamId);
    return this.threadStore.complete(streamId, threadId);
  }

  renameThread(streamId: string, threadId: string, title: string): Thread {
    this.resolveStream(streamId);
    return this.threadStore.rename(streamId, threadId, title);
  }

  setAutoCommit(streamId: string, threadId: string, enabled: boolean): Thread[] {
    this.resolveThread(streamId, threadId);
    return this.threadStore.setAutoCommit(threadId, enabled);
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
    // reset of "working" → "done".
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

  private buildRecentHumanCheckReminder(threadId: string): string {
    return buildRecentHumanCheckReminder(this.workItemStore.listItems(threadId), Date.now());
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
   * Called by the `oxplow__read_work_options` MCP handler so the runtime
   * can suppress the ready-work Stop directive on the very next Stop
   * when the set the agent just saw is unchanged. The record is
   * consumed (deleted) by the first Stop hook that reads it — a second
   * Stop on the same thread without a fresh read_work_options call
   * fires normally.
   */
  markReadWorkOptions(threadId: string, readyIds: string[]): void {
    this.lastReadWorkOptionsByThread.set(threadId, [...readyIds]);
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
    }
    if (envelope.event === "UserPromptSubmit") {
      const focusContext = formatEditorFocusForAgent(this.editorFocusStore.get(streamId));
      // If an agent-authored human_check item was closed on this thread
      // very recently, there's a strong chance this new prompt is either
      // a redo on it or a follow-up concern. Inject a reminder pointing
      // at the item so the agent knows to reopen (update_work_item →
      // in_progress) rather than silently expand scope or file a
      // duplicate "Fix …" task. See CLAUDE.md "Fixes/redos on a just-
      // shipped item" and the redoHint on create_work_item.
      const redoReminder = envelope.threadId
        ? this.buildRecentHumanCheckReminder(envelope.threadId)
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
      const additionalContext = [sessionContext, focusContext, redoReminder].filter(Boolean).join("\n\n");
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
   * side effects (e.g. flipping a wait point to `triggered`), and
   * returns the hook body for Claude. The decision logic itself lives
   * in `src/electron/stop-hook-pipeline.ts` so each branch is unit-
   * testable without spinning up a runtime.
   */
  private computeStopDirective(threadId: string): Record<string, unknown> | null {
    const thread = this.threadStore.findById(threadId);
    // Auto-commit (both ad-hoc via thread.auto_commit and manually-placed
    // mode=auto commit points) is now routed through the agent via a
    // Stop-hook directive — see buildAutoCommitStopReason. The runtime no
    // longer generates a message mechanically; the agent inspects the diff
    // and calls `mcp__oxplow__commit`. That unifies the approve-mode and
    // auto-mode flows: the only remaining distinction is "ask the user
    // first, yes/no." See .context/agent-model.md for the flow.
    // Consume the just-read record (once per call). If the agent called
    // read_work_options during the turn we're closing and the set matches
    // what's currently ready, the pipeline suppresses the ready-work
    // directive — the agent already has the list.
    const justReadReadySet = this.lastReadWorkOptionsByThread.get(threadId);
    this.lastReadWorkOptionsByThread.delete(threadId);
    // Cheap clean-tree probe: suppresses the auto-commit directive when an
    // ad-hoc `git commit` (Bash / Files panel) already landed the work, so
    // the Stop hook doesn't refire a no-op commit directive. See
    // wi-ec4c8e6f44fd. Read from the thread's worktree; null / unknown
    // paths fall back to "assume dirty" so we don't accidentally mute a
    // legit commit directive.
    let worktreeClean = false;
    const stream = thread ? this.store.get(thread.stream_id) ?? null : null;
    if (stream) {
      try { worktreeClean = isWorktreeClean(stream.worktree_path); } catch (err) {
        this.logger.debug("isWorktreeClean probe failed", {
          threadId, error: err instanceof Error ? err.message : String(err),
        });
      }
    }
    const snapshot: ThreadSnapshot = {
      thread,
      commitPoints: this.commitPointStore.listForThread(threadId),
      waitPoints: this.waitPointStore.listForThread(threadId),
      workItems: this.workItemStore.listItems(threadId),
      readyWorkItems: this.workItemStore.listReady(threadId),
      autoCommit: thread?.auto_commit ?? false,
      justReadReadySet,
      worktreeClean,
      subagentInFlight: (this.pendingSubagentsByThread.get(threadId) ?? 0) > 0,
      lastInProgressAuditSignature: this.lastAuditSignatureByThread.get(threadId),
    };
    // The item's own thread_id is what matters for the directive text (not
    // `threadId` — they agree today but could diverge if listReady ever
    // returns cross-thread candidates). stream_id comes off the thread row.
    const streamId = thread?.stream_id ?? "";
    const outcome = decideStopDirective(snapshot, {
      buildCommitPointReason: buildCommitPointStopReason,
      buildAutoCommitReason: buildAutoCommitStopReason,
      // item.thread_id is typed nullable (WorkItem covers backlog items too),
      // but decideStopDirective only emits this reason for in-thread rows, so
      // a fall-back to `threadId` keeps the directive stable.
      buildNextWorkItemReason: (item) =>
        buildNextWorkItemStopReason({ ...item, thread_id: item.thread_id ?? threadId }, streamId),
      buildInProgressAuditReason: buildInProgressAuditStopReason,
    });
    for (const effect of outcome.sideEffects) {
      if (effect.kind === "trigger-wait-point") {
        try { this.waitPointStore.trigger(effect.id); } catch (err) {
          this.logger.warn("trigger-wait-point side effect failed", {
            id: effect.id,
            error: err instanceof Error ? err.message : String(err),
          });
        }
      } else if (effect.kind === "record-audit-signature") {
        this.lastAuditSignatureByThread.set(threadId, effect.signature);
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
  }

  // -------- commit points (IPC-exposed delegations) --------

  listCommitPoints(threadId: string): CommitPoint[] {
    return this.threadQueue.listCommitPoints(threadId);
  }

  createCommitPoint(streamId: string, threadId: string): CommitPoint {
    this.resolveThread(streamId, threadId);
    return this.threadQueue.createCommitPoint(threadId);
  }

  deleteCommitPoint(id: string): void {
    this.threadQueue.deleteCommitPoint(id);
  }

  updateCommitPoint(id: string, changes: { mode?: "auto" | "approve" }): CommitPoint[] {
    return this.threadQueue.updateCommitPoint(id, changes);
  }

  /** IPC-exposed: run the git commit for a commit point immediately. */
  commitCommitPoint(id: string, message: string): CommitPoint {
    return this.threadQueue.executeCommit(id, message);
  }

  /**
   * Execute an agent-drafted auto-commit for a thread (no commit_point
   * row). Called from the `mcp__oxplow__commit` tool when the agent
   * passes `{ auto: true, message }`. Runs `git commit` synchronously,
   * publishes the `auto-committed` thread lifecycle event, and throws
   * on failure so the agent sees the stderr and can retry. Falls back
   * to `buildAutoCommitMessage` when `message` is empty/missing.
   */
  executeAutoCommitForThread(threadId: string, message: string | undefined): { sha: string; message: string } {
    const thread = this.threadStore.findById(threadId);
    if (!thread) throw new Error(`unknown thread: ${threadId}`);
    if (thread.status !== "active") throw new Error(`thread ${threadId} is not the writer; only active threads can commit`);
    const stream = this.store.get(thread.stream_id);
    if (!stream) throw new Error(`thread ${threadId} has no stream`);
    const finalMessage = (message && message.trim().length > 0)
      ? message
      : (() => {
          const workItems = this.workItemStore.listItems(threadId);
          const latestDone = this.commitPointStore.getLatestDoneForThread(threadId);
          return buildAutoCommitMessage(workItems, latestDone?.completed_at ?? null);
        })();
    const result = gitCommitAll(stream.worktree_path, finalMessage, { includeUntracked: true });
    if (!result.ok || !result.sha) {
      throw new Error(`git commit failed: ${result.stderr || "unknown"}`);
    }
    this.logger.info("auto-commit: committed", { threadId, sha: result.sha, message: finalMessage });
    this.events.publish({
      type: "thread.changed",
      streamId: thread.stream_id,
      threadId: thread.id,
      kind: "auto-committed",
    });
    return { sha: result.sha, message: finalMessage };
  }

  reorderThreadQueue(
    streamId: string,
    threadId: string,
    entries: Array<{ kind: "work" | "commit" | "wait"; id: string }>,
  ): void {
    this.resolveThread(streamId, threadId);
    this.threadQueue.reorderThreadQueue(threadId, entries);
  }

  // -------- wait points (IPC-exposed delegations) --------

  listWaitPoints(threadId: string): WaitPoint[] {
    return this.threadQueue.listWaitPoints(threadId);
  }

  createWaitPoint(streamId: string, threadId: string, note?: string | null): WaitPoint {
    this.resolveThread(streamId, threadId);
    return this.threadQueue.createWaitPoint(threadId, note ?? null);
  }

  setWaitPointNote(id: string, note: string | null): WaitPoint {
    return this.threadQueue.setWaitPointNote(id, note);
  }

  deleteWaitPoint(id: string): void {
    this.threadQueue.deleteWaitPoint(id);
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

export function buildNextWorkItemStopReason(
  item: { id: string; title: string; kind: string; thread_id: string },
  _streamId: string,
): string {
  return `The thread queue has ready work (threadId="${item.thread_id}"). Call \`mcp__oxplow__read_work_options\` and dispatch to a \`general-purpose\` subagent per the oxplow-runtime skill.`;
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
    `- If its acceptance criteria are met, call \`mcp__oxplow__complete_task\` (status \`human_check\`) — never self-mark \`done\`.`,
    `- If you're stuck and need a user decision, \`mcp__oxplow__update_work_item\` with \`status: "blocked"\`.`,
    `- If it's paused but resumable later, \`status: "ready"\`.`,
    `- If it's no longer relevant, \`status: "canceled"\`.`,
    ``,
    `When referring to any of these items to the user in chat, use the quoted title — never the \`wi-…\` id. The id is internal to tool calls.`,
  ].join("\n");
}

/**
 * Fallback auto-commit message builder used only when the agent-drafted
 * path fails (no message provided to `mcp__oxplow__commit`). The primary
 * path is agent-drafted via the Stop-hook directive — this keeps the
 * runtime able to land *something* rather than stall.
 *
 * `previousCommitCompletedAt` bounds the set of work items to those
 * whose `updated_at` is strictly after that moment — i.e., items that
 * settled since the previous commit landed. Passing `null` disables the
 * filter (first-commit case). Without this bound the message
 * monotonically re-counted every settled item in the thread, so each
 * auto-commit produced "complete N work items" with N climbing forever
 * and a body of whichever titles sorted first, regardless of what
 * actually changed.
 */
export function buildAutoCommitMessage(
  workItems: WorkItem[],
  previousCommitCompletedAt: string | null = null,
): string {
  const settled = workItems.filter((item) => {
    if (
      item.status !== "human_check" &&
      item.status !== "done" &&
      item.status !== "canceled"
    ) {
      return false;
    }
    if (!previousCommitCompletedAt) return true;
    return item.updated_at > previousCommitCompletedAt;
  });
  if (settled.length === 0) {
    return "chore: auto-commit at queue commit point";
  }
  if (settled.length === 1) {
    return `chore: ${settled[0]!.title}`;
  }
  const items = settled
    .slice(0, 5)
    .map((item) => `- ${item.title}`)
    .join("\n");
  const suffix = settled.length > 5 ? `\n…and ${settled.length - 5} more` : "";
  return `chore: complete ${settled.length} work items\n\n${items}${suffix}`;
}

/**
 * Directive text for auto-commit — identical to the approve-mode variant
 * minus the "ask the user to approve" gate. The agent inspects the diff,
 * drafts a message, and commits in one turn. Used both when
 * `thread.auto_commit=true` triggers an ad-hoc commit (no DB row) and
 * when a manually-placed commit_point row has `mode="auto"`.
 *
 * Passing `commit_point_id: null` selects the no-row shape; the agent
 * calls `mcp__oxplow__commit` with `{ auto: true, message }`. With a
 * commit point id, the agent passes it so the row flips to done.
 */
export function buildAutoCommitStopReason(cp: CommitPoint | null): string {
  const commitArgs = cp
    ? `{ commit_point_id: "${cp.id}", message: "<final message>" }`
    : `{ auto: true, message: "<final message>" }`;
  const lines = [
    `Auto-commit is due in this thread${cp ? ` (commit_point_id=${cp.id}, mode=auto)` : ""}. Inspect the unstaged/staged changes with read-only git commands (\`git status\`, \`git diff\`, \`git diff --staged\`), draft a concise commit message from what you see, then call \`mcp__oxplow__commit\` with ${commitArgs}.`,
    ``,
    `Your own memory of this turn's work is the primary source; if you've lost context of earlier completed tasks that should be part of this commit, call \`mcp__oxplow__tasks_since_last_commit\` for supplementary context. The diff is still the source of truth — don't list items that aren't represented in the diff.`,
    ``,
    `Follow the repo's commit-message conventions (see CLAUDE.md or your user memory) for style. This is auto-commit — do NOT ask the user to approve first; commit in this turn.`,
  ];
  return lines.join("\n");
}

export function buildCommitPointStopReason(cp: CommitPoint): string {
  const lines = [
    `A commit point is due in this thread's work queue (commit_point_id=${cp.id}).`,
    ``,
    `Inspect the unstaged/staged changes since the last commit using read-only commands (\`git status\`, \`git diff\`, \`git diff --staged\`), then draft a concise commit message describing those changes. Follow the repo's commit-message conventions (see CLAUDE.md or your user memory) for style.`,
    ``,
    `Output the drafted message in your chat reply and ask the user to approve or suggest changes. Do NOT run \`git add\` or \`git commit\` yourself.`,
    ``,
    `When the user approves, call \`mcp__oxplow__commit\` with { commit_point_id: "${cp.id}", message: "<final message>" } — that runs the git commit. If the user suggests changes, redraft the message in your next reply and ask again; only call \`mcp__oxplow__commit\` once the user has explicitly approved.`,
  ];
  return lines.join("\n");
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
    `When you realize you're about to change project files and aren't already on a work item, file one with status \`in_progress\` and track your work against it. Pick the shape by structure, not by whether you planned it: \`mcp__oxplow__create_work_item\` with kind \`task\` for one coherent change (even if it spans a few files); \`mcp__oxplow__file_epic_with_children\` when the work has ≥3 sub-steps a reviewer would check off independently (distinct phases, handoffs, or subsystems). Test: could a child close to \`human_check\` on its own and have the user inspect just that piece? If no, it's one task — plenty of plans describe single tasks. No "auto" placeholder items — commit to a real, durable row. **When the work (or each epic child) actually ships, close that row in the same turn** via \`mcp__oxplow__complete_task\` (normal finish; pass \`touchedFiles\` with paths you edited so Local History can attribute writes) or \`update_work_item\` with \`status: "blocked"\` (need a user decision). It's fine for an item to stay \`in_progress\` across turns when you're mid-flight or waiting on a user question — just don't leave finished work parked there. Load the oxplow-runtime skill for tool details.`,
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
 * When the most-recently-closed agent-authored item on this thread is in
 * `human_check` and closed within the reminder window (default 15 min),
 * produce a prominent reminder pointing at it. Injected into
 * UserPromptSubmit additionalContext so that when the user's new prompt
 * is likely a correction to that item, the agent sees the reopen path
 * (update_work_item → in_progress) before anything else — instead of
 * filing a duplicate "Fix …" task, silently expanding another item's
 * scope, or forgetting to re-record the effort. Returns empty when
 * no eligible item exists.
 */
export function buildRecentHumanCheckReminder(
  items: WorkItem[],
  now: number,
  windowMs = 15 * 60 * 1000,
): string {
  const cutoff = now - windowMs;
  let candidate: { id: string; title: string; ts: number } | null = null;
  for (const item of items) {
    if (item.status !== "human_check") continue;
    if (item.author !== "agent") continue;
    const ts = Date.parse(item.updated_at);
    if (!Number.isFinite(ts) || ts < cutoff) continue;
    if (!candidate || ts > candidate.ts) {
      candidate = { id: item.id, title: item.title, ts };
    }
  }
  if (!candidate) return "";
  return [
    "<recent-human-check-reminder>",
    `You just closed "${candidate.title}" to human_check on this thread.`,
    "If the user's new prompt is a fix/redo/pushback on THAT item (even indirectly — \"still doesn't work\", \"that's wrong\", \"try again\", \"no\", etc.):",
    `  1. Call update_work_item itemId="${candidate.id}" status=in_progress to reopen it.`,
    "  2. Do the new effort in the same item.",
    "  3. Call complete_task back to human_check when done (with touchedFiles).",
    "Do NOT file a new \"Fix …\" task for the redo — that fragments history and the Work panel lies about how many concerns were actually raised.",
    "If the new prompt is a GENUINELY separate concern, file a new item as usual and ignore this reminder.",
    "When you mention this item to the user in chat, refer to it by its quoted title — never by its `wi-…` id. The id is internal to tool calls; the user doesn't see or know it.",
    "</recent-human-check-reminder>",
  ].join("\n");
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
    // 5-minute gap rule: skip the end-of-effort snapshot when the
    // stream's most recent snapshot is younger than the threshold.
    // Leaves effort.end_snapshot_id null; computeEffortFiles already
    // returns null for an open-ended effort, so the History view will
    // wait until the next snapshot lands.
    const lastSnapTs = deps.getMostRecentSnapshotTimestamp?.() ?? null;
    const skipEnd = lastSnapTs !== null && shouldSkipEndSnapshot(lastSnapTs, Date.now());
    const endSnapshotId = skipEnd
      ? null
      : deps.flushSnapshot("task-end", { effortId: openEffort?.id ?? null });
    deps.effortStore.closeEffort({ workItemId, endSnapshotId });
    if (openEffort && (next === "human_check" || next === "blocked") && Array.isArray(touchedFiles) && touchedFiles.length > 0) {
      // Dedup, then enforce the cap. Oversized payloads drop ALL rows
      // so computeEffortFiles falls back to raw pair-diff ("assume all").
      const deduped = Array.from(new Set(touchedFiles.filter((p) => typeof p === "string" && p.length > 0)));
      if (deduped.length > 0 && deduped.length <= TOUCHED_FILES_CAP) {
        for (const path of deduped) {
          deps.effortStore.recordEffortFile(openEffort.id, path);
        }
      }
    }
  }
}

/**
 * Per-effort file list. Computes the pair-diff over
 * (start_snapshot_id, end_snapshot_id); when 2+ efforts end at the same
 * snapshot AND this effort has ≥1 row in `work_item_effort_file`, the
 * result is filtered to those paths so parallel subagents each see only
 * their own writes. If this effort has zero rows (agent skipped
 * `touchedFiles` on the human_check transition, or list exceeded the
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
  "bun test", "bunx tsc",
]);

/**
 * Decide whether a tool call should trigger the auto-file path. The
 * whitelist in ALWAYS_WRITE_INTENT_TOOLS is hard; Bash is write-intent
 * unless its command starts with an obvious read-only verb. False
 * positives are cheap (an extra auto-filed item) — we err toward
 * write-intent when unsure so we don't miss genuine edits.
 */
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
