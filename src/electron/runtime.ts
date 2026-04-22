import { EventEmitter } from "node:events";
import { watch, type FSWatcher } from "node:fs";
import { join, resolve, sep } from "node:path";
import { randomUUID } from "node:crypto";
import { buildAgentCommandForSession } from "../agent/agent-command.js";
import { buildWriteGuardResponse, NON_WRITER_PROMPT_BLOCK } from "./write-guard.js";
import { decideStopDirective, type BatchSnapshot } from "./stop-hook-pipeline.js";
import { BatchQueueOrchestrator } from "./batch-queue-orchestrator.js";
import { ensureAgentPane } from "../terminal/fleet.js";
import { BatchStore, type Batch, type BatchState } from "../persistence/batch-store.js";
import {
  detectBaseBranch,
  ensureWorktree,
  isGitRepo,
  isGitWorktree,
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
  listFileCommits,
  gitBlame,
  listAllRefs,
  type BranchChanges,
  type ChangeScopes,
  type CommitDetail,
  type GitLogCommit,
  type GitOpResult,
  type RefOption,
  type TextSearchHit,
} from "../git/git.js";
import { HookEventStore, ingestHookPayload } from "../session/hook-ingest.js";
import { EditorFocusStore, formatEditorFocusForAgent, type EditorFocusState } from "../session/editor-focus.js";
import { deriveBatchAgentStatus, type AgentStatus } from "../session/agent-status.js";
import { LspSessionManager, registerLanguageServer } from "../lsp/lsp.js";
import { createUiClientLogger, createDaemonLogger, type Logger, type LogLevel } from "../core/logger.js";
import { decideResumeUpdate } from "../session/resume-tracker.js";
import { readTurnUsage } from "../session/transcript-usage.js";
import { createElectronPlugin, HOOK_EVENTS, type ElectronPlugin } from "../session/claude-plugin.js";
import { startMcpServer, type HookEnvelope, type McpServerHandle } from "../mcp/mcp-server.js";
import { buildWorkItemMcpTools } from "../mcp/mcp-tools.js";
import { buildLspMcpTools } from "../mcp/lsp-mcp-tools.js";
import { getStateDatabase } from "../persistence/state-db.js";
import { StreamStore, type PaneKind, type Stream } from "../persistence/stream-store.js";
import { BACKLOG_SCOPE, WorkItemStore, type WorkItem } from "../persistence/work-item-store.js";
import { CommitPointStore, type CommitPoint } from "../persistence/commit-point-store.js";
import { WaitPointStore, type WaitPoint } from "../persistence/wait-point-store.js";
import { TurnStore, type AgentTurn } from "../persistence/turn-store.js";
import { WorkItemEffortStore } from "../persistence/work-item-effort-store.js";
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
import { EventBus, type NewdeEvent } from "../core/event-bus.js";
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
import { detectCurrentBranch } from "../git/git.js";
import { loadProjectConfig, writeProjectConfig, type NewdeConfig } from "../config/config.js";
import { killSession } from "../terminal/tmux.js";
import { attachPane } from "../terminal/pty-bridge.js";
import { AgentPtyStore } from "../terminal/agent-pty-store.js";
import type { EditorFocusPayload, UiLogPayload } from "./ipc-contract.js";

export class ElectronRuntime {
  readonly projectDir: string;
  readonly projectBase: string;
  readonly logger: Logger;
  readonly store: StreamStore;
  readonly batchStore: BatchStore;
  private readonly workItemStore: WorkItemStore;
  readonly commitPointStore: CommitPointStore;
  readonly waitPointStore: WaitPointStore;
  readonly batchQueue: BatchQueueOrchestrator;
  readonly turnStore: TurnStore;
  readonly effortStore: WorkItemEffortStore;
  readonly snapshotStore: SnapshotStore;
  readonly workItemApi: WorkItemApi;
  readonly hookEvents: HookEventStore;
  readonly lspManager: LspSessionManager;
  readonly editorFocusStore: EditorFocusStore;
  readonly agentPtyStore: AgentPtyStore;
  readonly workspaceWatchers: WorkspaceWatcherRegistry;
  readonly gitRefsWatchers: GitRefsWatcherRegistry;
  config: NewdeConfig;
  readonly events: EventBus;

  private electronPlugin: ElectronPlugin | null = null;
  private readonly terminalSessions = new Map<string, RuntimeSocket>();
  private readonly lspClients = new Map<string, RuntimeSocket>();
  private readonly agentStatusByBatch = new Map<string, AgentStatus>();
  private readonly recentUiWrites = new Map<string, number>();
  private readonly dirtyPathsByStream = new Map<string, Set<string>>();
  /** Last <session-context> block we sent to each Claude session. Used to
   *  skip re-injecting identical blocks turn-over-turn. Keyed by Claude
   *  session id; absent key means "nothing sent yet / fall back to emit". */
  private readonly lastSessionContextBySessionId = new Map<string, string>();
  /** The batch's writer/read-only role at the moment a Claude session id was
   *  first seen. Captured once and never rewritten, so
   *  buildSessionContextBlock can detect a mid-session role flip and emit a
   *  loud banner superseding the frozen NON_WRITER block in the agent's
   *  initial system prompt. Keyed by Claude session id. */
  private readonly initialRoleBySessionId = new Map<string, "writer" | "read-only">();
  private mcp: McpServerHandle | null = null;
  private gitEnabledCached = false;
  private gitRootWatcher: FSWatcher | null = null;
  private snapshotCleanupTimer: ReturnType<typeof setInterval> | null = null;
  private disposed = false;

  private constructor(projectDir: string, projectBase: string, logger: Logger, config: NewdeConfig) {
    this.projectDir = projectDir;
    this.projectBase = projectBase;
    this.logger = logger;
    this.config = config;
    this.store = new StreamStore(projectDir, logger.child({ subsystem: "stream-store" }));
    this.batchStore = new BatchStore(projectDir, logger.child({ subsystem: "batch-store" }));
    this.workItemStore = new WorkItemStore(projectDir, logger.child({ subsystem: "work-items" }));
    this.commitPointStore = new CommitPointStore(projectDir, logger.child({ subsystem: "commit-points" }));
    this.waitPointStore = new WaitPointStore(projectDir, logger.child({ subsystem: "wait-points" }));
    this.batchQueue = new BatchQueueOrchestrator(
      this.store,
      this.batchStore,
      this.workItemStore,
      this.commitPointStore,
      this.waitPointStore,
      logger.child({ subsystem: "batch-queue" }),
    );
    this.turnStore = new TurnStore(projectDir, logger.child({ subsystem: "turn-store" }));
    this.effortStore = new WorkItemEffortStore(projectDir, logger.child({ subsystem: "effort-store" }));
    this.snapshotStore = new SnapshotStore(projectDir, logger.child({ subsystem: "snapshot-store" }));
    this.snapshotStore.setMaxFileBytes(config.snapshotMaxFileBytes);
    this.workItemApi = createWorkItemApi({
      resolveBatch: (streamId, batchId) => this.resolveBatch(streamId, batchId),
      workItemStore: this.workItemStore,
      turnStore: this.turnStore,
      effortStore: this.effortStore,
      snapshotStore: this.snapshotStore,
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
    // newde manages its own worktrees under .newde/worktrees/; refusing to
    // boot inside someone else's worktree keeps the stream/pane accounting
    // from getting tangled with a foreign git checkout.
    if (isGitWorktree(projectDir)) {
      throw new Error(
        `newde cannot run inside a git worktree (${projectDir}). Open it from the main repository checkout or from a directory that isn't under git.`,
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
    const gitWorkspace = isGitRepo(this.projectDir);
    const branch = gitWorkspace ? detectCurrentBranch(this.projectDir) ?? this.projectBase : this.projectBase;

    let stream = this.store.findByBranch(branch);
    if (!stream) {
      stream = this.store.create({
        title: branch,
        branch,
        branchRef: gitWorkspace ? `refs/heads/${branch}` : branch,
        branchSource: "local",
        worktreePath: this.projectDir,
        projectBase: this.projectBase,
      });
      this.logger.info("created initial stream", { streamId: stream.id, branch });
    } else {
      this.logger.info("reusing initial stream", { streamId: stream.id, branch });
    }

    this.store.ensureCurrentStreamId(stream.id);
    cleanupSessions(this.store.list());
    this.logger.info("initialized current stream", { streamId: this.store.getCurrentStreamId() });

    // Push the user's `generatedDirs` down to the watcher before starting
    // any stream-scoped watchers so they pick up the filter from the first
    // event.
    this.workspaceWatchers.setExtraIgnoreDirs(this.config.generatedDirs);

    for (const existingStream of this.store.list()) {
      this.batchStore.ensureStream(existingStream);
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
      // Track into the snapshot dirty set regardless of active batch state —
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
    });
    this.hookEvents.subscribe((event) => {
      this.events.publish({
        type: "hook.recorded",
        streamId: event.streamId,
        batchId: event.batchId,
        pane: event.pane,
        event,
      });
      if (event.batchId) this.recomputeAgentStatus(event.streamId, event.batchId);
    });
    this.workItemStore.subscribe((change) => {
      if (change.batchId === BACKLOG_SCOPE) {
        this.events.publish({
          type: "backlog.changed",
          kind: change.kind,
          itemId: change.itemId,
        });
        return;
      }
      const batch = this.batchStore.findById(change.batchId);
      if (!batch) return;
      if (change.kind === "updated" && change.itemId && change.previousStatus !== change.nextStatus) {
        this.handleStatusTransition(
          batch.stream_id,
          change.batchId,
          change.itemId,
          change.previousStatus,
          change.nextStatus,
          change.touchedFiles,
        );
      }
      this.events.publish({
        type: "work-item.changed",
        streamId: batch.stream_id,
        batchId: change.batchId,
        kind: change.kind,
        itemId: change.itemId,
      });
    });
    this.batchStore.subscribe((change) => {
      this.events.publish({
        type: "batch.changed",
        streamId: change.streamId,
        batchId: change.batchId,
        kind: change.kind,
      });
    });
    this.store.subscribe((change) => {
      this.events.publish({ type: "stream.changed", kind: change.kind, streamId: change.streamId });
    });
    this.waitPointStore.subscribe((change) => {
      const batch = this.batchStore.findById(change.batchId);
      this.events.publish({
        type: "wait-point.changed",
        streamId: batch?.stream_id ?? null,
        batchId: change.batchId,
        id: change.id,
        kind: change.kind,
      });
    });
    this.commitPointStore.subscribe((change) => {
      const batch = this.batchStore.findById(change.batchId);
      this.events.publish({
        type: "commit-point.changed",
        streamId: batch?.stream_id ?? null,
        batchId: change.batchId,
        id: change.id,
        kind: change.kind,
      });
    });
    this.turnStore.subscribe((change) => {
      const batch = this.batchStore.findById(change.batchId);
      if (!batch) return;
      this.events.publish({
        type: "turn.changed",
        streamId: batch.stream_id,
        batchId: change.batchId,
        turnId: change.turnId,
        kind: change.kind,
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
          resolveBatchById: (batchId) => this.resolveBatchById(batchId),
          batchStore: this.batchStore,
          streamStore: this.store,
          workItemStore: this.workItemStore,
          commitPointStore: this.commitPointStore,
          executeCommit: (cpId, message) => this.batchQueue.executeCommit(cpId, message),
          turnStore: this.turnStore,
          waitPointStore: this.waitPointStore,
        }),
        ...buildLspMcpTools({
          resolveStream: (streamId) => this.resolveStream(streamId),
          lspManager: this.lspManager,
        }),
      ],
      onHook: (envelope) => this.handleHookEnvelope(envelope),
    });
    this.logger.info("started mcp server", { port: this.mcp.port, lockfilePath: this.mcp.lockfilePath });
  }

  async dispose(): Promise<void> {
    this.disposed = true;
    this.gitRootWatcher?.close();
    this.gitRootWatcher = null;
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
    await this.lspManager.dispose();
    if (this.mcp) {
      await this.mcp.stop();
      this.mcp = null;
    }
    getStateDatabase(this.projectDir).close();
  }

  onEvent(listener: (event: NewdeEvent) => void): () => void {
    return this.events.subscribe(listener);
  }

  getBatchAgentStatus(batchId: string): AgentStatus {
    return this.agentStatusByBatch.get(batchId) ?? "idle";
  }

  listAgentStatuses(streamId?: string): Array<{ streamId: string; batchId: string; status: AgentStatus }> {
    const out: Array<{ streamId: string; batchId: string; status: AgentStatus }> = [];
    for (const [batchId, status] of this.agentStatusByBatch) {
      const batch = this.batchStore.findById(batchId);
      if (!batch) continue;
      if (streamId && batch.stream_id !== streamId) continue;
      out.push({ streamId: batch.stream_id, batchId, status });
    }
    return out;
  }

  private recomputeAgentStatus(streamId: string, batchId: string): void {
    const events = this.hookEvents.list(streamId).filter((candidate) => candidate.batchId === batchId);
    const next = deriveBatchAgentStatus(events);
    const prev = this.agentStatusByBatch.get(batchId);
    if (prev === next) return;
    this.agentStatusByBatch.set(batchId, next);
    this.events.publish({
      type: "agent-status.changed",
      streamId,
      batchId,
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

  getConfig(): NewdeConfig {
    return this.config;
  }

  setAgentPromptAppend(text: string): NewdeConfig {
    const next: NewdeConfig = { ...this.config, agentPromptAppend: text };
    writeProjectConfig(this.projectDir, next);
    this.config = next;
    this.logger.info("updated agent prompt append", { length: text.length });
    this.events.publish({ type: "config.changed" });
    return next;
  }

  setSnapshotRetentionDays(days: number): NewdeConfig {
    if (!Number.isFinite(days) || days < 0) {
      throw new Error("snapshotRetentionDays must be a non-negative number");
    }
    const next: NewdeConfig = { ...this.config, snapshotRetentionDays: days };
    writeProjectConfig(this.projectDir, next);
    this.config = next;
    this.logger.info("updated snapshot retention days", { days });
    this.events.publish({ type: "config.changed" });
    return next;
  }

  setSnapshotMaxFileBytes(bytes: number): NewdeConfig {
    if (!Number.isFinite(bytes) || bytes < 1024) {
      throw new Error("snapshotMaxFileBytes must be a number >= 1024");
    }
    const next: NewdeConfig = { ...this.config, snapshotMaxFileBytes: Math.floor(bytes) };
    writeProjectConfig(this.projectDir, next);
    this.config = next;
    this.logger.info("updated snapshot max file bytes", { bytes: next.snapshotMaxFileBytes });
    this.events.publish({ type: "config.changed" });
    return next;
  }

  setGeneratedDirs(dirs: string[]): NewdeConfig {
    // Normalize: strip leading/trailing slashes, dedupe, drop empties. Path
    // separators are illegal — single path segments only, per config schema.
    const normalized = Array.from(
      new Set(
        dirs
          .map((entry) => entry.trim().replace(/^\/+|\/+$/g, ""))
          .filter((entry) => entry.length > 0 && !entry.includes("/")),
      ),
    ).sort();
    const next: NewdeConfig = { ...this.config, generatedDirs: normalized };
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
   * Per-line blame combining newde work-item efforts (authoritative) with
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

  createStream(body: { title: string; summary?: string; source: "existing"; ref: string } | { title: string; summary?: string; source: "new"; branch: string; startPointRef: string }): Stream {
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
    } else {
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
    }
    this.workspaceWatchers.ensureWatching(stream);
    this.gitRefsWatchers.ensureWatching(stream);
    this.batchStore.ensureStream(stream);
    this.store.setCurrentStreamId(stream.id);
    return stream;
  }

  getBatchState(streamId: string): BatchState {
    const stream = this.resolveStream(streamId);
    return this.batchStore.ensureStream(stream);
  }

  createBatch(streamId: string, title: string): BatchState {
    const stream = this.resolveStream(streamId);
    if (!title.trim()) throw new Error("batch title is required");
    return this.batchStore.create(stream, { title });
  }

  reorderBatch(streamId: string, batchId: string, targetIndex: number): BatchState {
    this.resolveStream(streamId);
    return this.batchStore.reorder(streamId, batchId, targetIndex);
  }

  reorderBatches(streamId: string, orderedBatchIds: string[]): void {
    this.resolveStream(streamId);
    this.batchStore.reorderBatches(streamId, orderedBatchIds);
  }

  reorderStreams(orderedStreamIds: string[]): void {
    this.store.reorderStreams(orderedStreamIds);
  }

  selectBatch(streamId: string, batchId: string): BatchState {
    this.resolveStream(streamId);
    return this.batchStore.select(streamId, batchId);
  }

  promoteBatch(streamId: string, batchId: string): BatchState {
    this.resolveStream(streamId);
    return this.batchStore.promote(streamId, batchId);
  }

  completeBatch(streamId: string, batchId: string): BatchState {
    this.resolveStream(streamId);
    return this.batchStore.complete(streamId, batchId);
  }

  renameBatch(streamId: string, batchId: string, title: string): Batch {
    this.resolveStream(streamId);
    return this.batchStore.rename(streamId, batchId, title);
  }

  setAutoCommit(streamId: string, batchId: string, enabled: boolean): Batch[] {
    this.resolveBatch(streamId, batchId);
    return this.batchStore.setAutoCommit(batchId, enabled);
  }

  setStreamPrompt(streamId: string, prompt: string | null): Stream[] {
    return this.store.setStreamPrompt(streamId, prompt);
  }

  setBatchPrompt(streamId: string, batchId: string, prompt: string | null): Batch[] {
    this.resolveBatch(streamId, batchId);
    return this.batchStore.setBatchPrompt(batchId, prompt);
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
    const batch = this.batchStore.findByPane(paneTarget);
    if (!batch) {
      throw new Error(`unknown pane target: ${paneTarget}`);
    }
    const stream = this.resolveStream(batch.stream_id);
    const paneLogger = this.logger.child({
      streamId: stream.id,
      batchId: batch.id,
      paneTarget,
    });
    const agentCommand = this.getAgentCommand(stream, batch);
    if (mode === "tmux") {
      // Use a resume-less variant as the launcher identity so reconnecting to
      // a live agent whose resume id has since changed doesn't look like a
      // config change and trigger a respawn.
      const signatureSource = this.getAgentCommand(stream, batch, { withoutResume: true });
      ensureAgentPane(
        batch.pane_target,
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
    });
    if (mode === "tmux") {
      attachPane(socket, batch.pane_target, cols, rows, paneLogger.child({ subsystem: "pty-bridge", mode }));
    } else {
      // Direct-mode agent PTYs live in the runtime and persist across
      // UI attach/detach. Switching batches or streams detaches the socket
      // but leaves the Claude process running so the user can return to an
      // in-progress agent without killing and resuming it.
      const agentPty = this.agentPtyStore.ensure(
        batch.id,
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
    socket.emit("message", message);
  }

  closeTerminalSession(sessionId: string): void {
    const socket = this.terminalSessions.get(sessionId);
    if (!socket) return;
    this.terminalSessions.delete(sessionId);
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

  private resolveBatch(streamId: string, batchId: string): Batch {
    this.resolveStream(streamId);
    const batch = this.batchStore.getBatch(streamId, batchId);
    if (!batch) throw new Error(`unknown batch: ${batchId}`);
    return batch;
  }

  // batchIds are globally unique, so a lookup by id alone is enough. MCP
  // tools use this when the caller omitted streamId (or passed one that
  // drifted out of sync with the UI's current stream); the agent's session
  // prompt shouldn't need to stay perfectly aligned with whatever stream
  // the user is viewing.
  private resolveBatchById(batchId: string): Batch {
    const batch = this.batchStore.findById(batchId);
    if (!batch) throw new Error(`unknown batch: ${batchId}`);
    return batch;
  }

  // Build a <session-context> additionalContext block reflecting LIVE state
  // (as opposed to the frozen snapshot in the agent's system prompt). Called
  // on every UserPromptSubmit — see handleHookEnvelope. Returns empty string
  // when the envelope lacks enough to resolve a batch.
  private buildRefreshedSessionContext(
    envelopeBatchId: string | null,
    streamId: string,
    sessionId: string | undefined,
  ): string {
    void streamId;
    const batch = envelopeBatchId ? this.batchStore.findById(envelopeBatchId) : null;
    if (!batch) return "";
    const stream = this.store.get(batch.stream_id);
    if (!stream) return "";
    const batchState = this.batchStore.list(stream.id);
    const activeBatch = batchState.batches.find((b) => b.id === batchState.activeBatchId) ?? null;
    // Stash (once) the role this batch had when Claude's session id was first
    // observed, so a later promotion/demotion surfaces as a ROLE CHANGE banner
    // rather than a subtle one-line diff.
    const currentRole: "writer" | "read-only" =
      activeBatch && activeBatch.id !== batch.id ? "read-only" : "writer";
    let initialRole: "writer" | "read-only" | undefined;
    if (sessionId) {
      if (!this.initialRoleBySessionId.has(sessionId)) {
        this.initialRoleBySessionId.set(sessionId, currentRole);
      }
      initialRole = this.initialRoleBySessionId.get(sessionId);
    }
    return buildSessionContextBlock({ stream, batch, activeBatch, initialRole });
  }

  private resolveActiveBatchForPrompt(streamId: string): Batch | null {
    const activeId = this.batchStore.list(streamId).activeBatchId;
    if (!activeId) return null;
    return this.batchStore.getBatch(streamId, activeId) ?? null;
  }

  private getAgentCommand(stream: Stream, batch: Batch, opts: { withoutResume?: boolean } = {}): string {
    const resumeSessionId = opts.withoutResume ? "" : batch.resume_session_id;
    if (this.config.agent === "claude") {
      if (!this.mcp) throw new Error("mcp server not started");
      // One Claude plugin per runtime (the MCP port + hook URL are stable for
      // the process's lifetime). Plugin hook JSON references env vars, so
      // per-batch identity flows in at exec time without re-writing files.
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
          allowedTools: ["mcp__newde__*"],
          appendSystemPrompt: buildBatchAgentPrompt(
            stream,
            batch,
            this.config.agentPromptAppend,
            this.resolveActiveBatchForPrompt(stream.id),
          ),
          mcpConfig: buildBatchMcpConfig(this.mcp),
          env: {
            NEWDE_STREAM_ID: stream.id,
            NEWDE_BATCH_ID: batch.id,
            NEWDE_HOOK_TOKEN: this.mcp.authToken,
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

  private handleHookEnvelope(envelope: HookEnvelope): { body?: unknown } | void {
    const streamId = envelope.streamId;
    if (!streamId) return;
    this.hookEventsSeen.add(envelope.event);
    const pane: PaneKind | undefined = envelope.pane === "working" || envelope.pane === "talking"
      ? envelope.pane
      : undefined;
    const stored = ingestHookPayload(this.hookEvents, envelope.event, envelope.payload, {
      streamId,
      batchId: envelope.batchId,
      pane,
    });
    if (envelope.batchId && this.store.get(streamId)) {
      const batch = this.batchStore.findById(envelope.batchId);
      const update = decideResumeUpdate(
        batch?.resume_session_id ?? "",
        stored.normalized.sessionId,
      );
      if (update) {
        this.batchStore.updateResume(streamId, envelope.batchId, update.sessionId);
      }
      this.applyTurnTracking(envelope, stored.normalized.sessionId);
    }
    if (envelope.event === "PreToolUse" && envelope.batchId) {
      // Fresh read of batch.status — promoting another batch to writer takes
      // effect on the next tool call without restarting any agent.
      const batch = this.batchStore.findById(envelope.batchId);
      const toolName = typeof (envelope.payload as { tool_name?: unknown })?.tool_name === "string"
        ? (envelope.payload as { tool_name: string }).tool_name
        : "";
      const deny = buildWriteGuardResponse(batch, toolName, {
        projectDir: this.projectDir,
        toolInput: (envelope.payload as { tool_input?: unknown })?.tool_input,
      });
      if (deny) return { body: deny };
    }
    if (envelope.event === "UserPromptSubmit") {
      const focusContext = formatEditorFocusForAgent(this.editorFocusStore.get(streamId));
      // Re-inject the session context each turn — the agent's system-prompt
      // SESSION CONTEXT line is frozen at launch, but the UI's active /
      // selected batch can flip mid-session. Reading the live state here
      // keeps the agent pointed at the right ids without a user-visible
      // prompt edit. Skip emission when the block is identical to what we
      // already sent on the same Claude session — the agent's prompt cache
      // still holds the prior value, so re-sending is pure overhead.
      let sessionContext = "";
      if (this.config.injectSessionContext) {
        const candidate = this.buildRefreshedSessionContext(envelope.batchId ?? null, streamId, stored.normalized.sessionId);
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
      const additionalContext = [sessionContext, focusContext].filter(Boolean).join("\n\n");
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
    if (envelope.event === "Stop" && envelope.batchId) {
      const directive = this.computeStopDirective(envelope.batchId);
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
  private computeStopDirective(batchId: string): Record<string, unknown> | null {
    const batch = this.batchStore.findById(batchId);
    // The Stop hook fires while the current turn is still "open" (closeTurn
    // runs inside applyTurnTracking's Stop branch alongside this). Grab its
    // started_at so decideStopDirective can skip items the agent filed for
    // triage during the turn. A missing/closed turn falls back to the
    // pre-fix behaviour.
    const openTurn = this.turnStore.currentOpenTurn(batchId);
    const currentTurnFilePaths = openTurn?.start_snapshot_id
      ? this.computeTurnFilePaths(openTurn)
      : [];
    // When auto-commit is on and the batch is the writer, run the commit
    // directly without synthesizing a commit_point row. No DB row means no
    // "Commit · Auto" noise in the queue UI; manually-placed auto-mode
    // commit points are handled by the branch below.
    if (batch?.auto_commit && batch.status === "active") {
      const workItems = this.workItemStore.listItems(batchId);
      const hasSettledWork = workItems.some(
        (item) => item.status === "human_check" || item.status === "done",
      );
      if (hasSettledWork) {
        const result = this.runAutoCommit(batch, workItems);
        if (result) {
          // Notify the UI that a commit landed in this batch's worktree so
          // downstream views (git log, Files panel) know to refresh. The
          // fs-watcher will also fire git-refs.changed, but surfacing it as
          // a batch-scoped lifecycle event keeps the batch UI responsive.
          this.events.publish({
            type: "batch.changed",
            streamId: batch.stream_id,
            batchId: batch.id,
            kind: "auto-committed",
          });
        }
      }
    }
    // Auto-mode commit points: commit immediately without blocking the agent.
    // If the active commit point has mode="auto" and is still pending, generate
    // a message from settled work items and commit right now. After committing
    // the point becomes "done", so the snapshot built below sees it as done
    // and decideStopDirective falls through to the next pipeline step.
    if (batch?.status === "active") {
      const commitPoints = this.commitPointStore.listForBatch(batchId);
      const workItems = this.workItemStore.listItems(batchId);
      const autoCommitPoint = findActiveAutoCommitPoint(commitPoints, workItems);
      if (autoCommitPoint) {
        this.executeAutoCommitPoint(autoCommitPoint, batch, workItems);
      }
    }
    const snapshot: BatchSnapshot = {
      batch,
      commitPoints: this.commitPointStore.listForBatch(batchId),
      waitPoints: this.waitPointStore.listForBatch(batchId),
      workItems: this.workItemStore.listItems(batchId),
      readyWorkItems: this.workItemStore.listReady(batchId),
      currentTurnStartedAt: openTurn?.started_at ?? null,
      currentTurnFilePaths,
      autoCommit: batch?.auto_commit ?? false,
    };
    // The item's own batch_id is what matters for the directive text (not
    // `batchId` — they agree today but could diverge if listReady ever
    // returns cross-batch candidates). stream_id comes off the batch row.
    const streamId = batch?.stream_id ?? "";
    const outcome = decideStopDirective(snapshot, {
      buildCommitPointReason: buildCommitPointStopReason,
      // item.batch_id is typed nullable (WorkItem covers backlog items too),
      // but decideStopDirective only emits this reason for in-batch rows, so
      // a fall-back to `batchId` keeps the directive stable.
      buildNextWorkItemReason: (item, context) =>
        buildNextWorkItemStopReason({ ...item, batch_id: item.batch_id ?? batchId }, streamId, context),
      buildHumanCheckNudgeReason: buildHumanCheckNudgeStopReason,
    });
    for (const effect of outcome.sideEffects) {
      if (effect.kind === "trigger-wait-point") {
        try { this.waitPointStore.trigger(effect.id); } catch (err) {
          this.logger.warn("trigger-wait-point side effect failed", {
            id: effect.id,
            error: err instanceof Error ? err.message : String(err),
          });
        }
      }
    }
    return outcome.directive ? { ...outcome.directive } : null;
  }

  private applyTurnTracking(envelope: HookEnvelope, sessionId: string | undefined): void {
    const batchId = envelope.batchId;
    if (!batchId) return;
    switch (envelope.event) {
      case "UserPromptSubmit": {
        const payload = (envelope.payload ?? {}) as { prompt?: unknown };
        const prompt = typeof payload.prompt === "string" ? payload.prompt : "";
        if (!prompt.trim()) return;
        // Defensive: if a prior turn never saw Stop, close it out so every
        // open turn corresponds to the latest prompt.
        const stillOpen = this.turnStore.currentOpenTurn(batchId);
        if (stillOpen) {
          this.turnStore.closeTurn(stillOpen.id, { answer: null });
        }
        const turn = this.turnStore.openTurn({ batchId, prompt, sessionId });
        const batch = this.batchStore.findById(batchId);
        if (batch) {
          const startSnapshotId = this.safeFlushSnapshot(batch.stream_id, "turn-start");
          if (startSnapshotId) this.turnStore.setStartSnapshot(turn.id, startSnapshotId);
          this.linkOpenEffortsToTurn(turn.id);
        }
        return;
      }
      case "PostToolUse": {
        const payload = (envelope.payload ?? {}) as {
          tool_name?: unknown;
          tool_input?: unknown;
          tool_response?: unknown;
        };
        const toolName = typeof payload.tool_name === "string" ? payload.tool_name : "";
        if (!FILE_EDIT_TOOLS.has(toolName)) return;
        const status = derivePostToolStatus(payload.tool_response);
        if (status === "error") return;
        const extractedPath = extractEditedFilePath(payload.tool_input);
        if (!extractedPath) return;
        const batch = this.batchStore.findById(batchId);
        const stream = batch ? this.store.get(batch.stream_id) ?? null : null;
        const normalizedPath = stream
          ? toWorktreeRelativePath(extractedPath, stream.worktree_path)
          : extractedPath;
        if (stream && !shouldAcceptHookFilePath(normalizedPath, stream.worktree_path, this.config.generatedDirs)) {
          return;
        }
        if (batch) this.markDirty(batch.stream_id, normalizedPath);
        // Per-effort write-log is populated on the status transition to
        // human_check via `update_work_item`'s `touchedFiles` payload — see
        // applyStatusTransition. The PostToolUse hook no longer guesses.
        return;
      }
      case "Stop": {
        const open = this.turnStore.currentOpenTurn(batchId);
        if (!open) return;
        const batch = this.batchStore.findById(batchId);
        this.turnStore.closeTurn(open.id, { answer: null });
        const transcriptPath = typeof (envelope.payload as { transcript_path?: unknown })?.transcript_path === "string"
          ? (envelope.payload as { transcript_path: string }).transcript_path
          : null;
        if (transcriptPath) {
          const usage = readTurnUsage(transcriptPath, open.started_at, this.logger);
          if (usage && (usage.inputTokens !== null || usage.outputTokens !== null || usage.cacheReadInputTokens !== null)) {
            this.turnStore.setTurnUsage(open.id, usage);
          }
        }
        if (batch) {
          const endSnapshotId = this.safeFlushSnapshot(batch.stream_id, "turn-end");
          if (endSnapshotId) this.turnStore.setEndSnapshot(open.id, endSnapshotId);
        }
        return;
      }
      case "SessionEnd": {
        const open = this.turnStore.currentOpenTurn(batchId);
        if (!open) return;
        this.turnStore.closeTurn(open.id, { answer: null });
        return;
      }
      default:
        return;
    }
  }

  /**
   * Auto-snapshot + effort bookkeeping for a work item status change.
   * `in_progress` opens a new effort with a start snapshot; any transition
   * out of `in_progress` closes it with an end snapshot.
   */
  private handleStatusTransition(
    streamId: string,
    batchId: string,
    workItemId: string,
    previous: WorkItem["status"] | undefined,
    next: WorkItem["status"] | undefined,
    touchedFiles?: string[],
  ): void {
    applyStatusTransition(
      {
        effortStore: this.effortStore,
        turnStore: this.turnStore,
        flushSnapshot: (source) => this.safeFlushSnapshot(streamId, source),
      },
      { batchId, workItemId, previous, next, touchedFiles },
    );
  }

  private linkOpenEffortsToTurn(turnId: string): void {
    linkOpenEffortsToTurn(this.effortStore, turnId);
  }

  private safeFlushSnapshot(streamId: string, source: SnapshotSource): string | null {
    try {
      return this.flushSnapshotForStream(streamId, source);
    } catch (error) {
      this.logger.warn("snapshot flush failed", {
        streamId,
        source,
        error: error instanceof Error ? error.message : String(error),
      });
      return null;
    }
  }

  /**
   * Returns paths that differ between the current open turn's start snapshot
   * and the current worktree state (used by the Stop-hook pipeline to know
   * which files the agent touched during this turn).
   */
  private computeTurnFilePaths(openTurn: AgentTurn): string[] {
    if (!openTurn.start_snapshot_id) return [];
    const batch = this.batchStore.findById(openTurn.batch_id);
    if (!batch) return [];
    const stream = this.store.get(batch.stream_id);
    if (!stream) return [];
    const ignore = (relpath: string) =>
      shouldIgnoreWorkspaceWatchPath(relpath, this.config.generatedDirs);
    return this.snapshotStore.reconcileWorktree(openTurn.start_snapshot_id, stream.worktree_path, ignore);
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
  private flushSnapshotForStream(streamId: string, source: SnapshotSource): string | null {
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
    });
    if (dirty) dirty.clear();
    if (result.created) {
      this.events.publish({
        type: "file-snapshot.created",
        streamId,
        snapshotId: result.id,
        kind: source,
        turnId: null,
        batchId: null,
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
            turnId: null,
            batchId: null,
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

  listAgentTurns(batchId: string, limit?: number): AgentTurn[] {
    return this.turnStore.listForBatch(batchId, limit);
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

  // -------- commit points (IPC-exposed delegations) --------

  listCommitPoints(batchId: string): CommitPoint[] {
    return this.batchQueue.listCommitPoints(batchId);
  }

  createCommitPoint(streamId: string, batchId: string): CommitPoint {
    this.resolveBatch(streamId, batchId);
    return this.batchQueue.createCommitPoint(batchId);
  }

  deleteCommitPoint(id: string): void {
    this.batchQueue.deleteCommitPoint(id);
  }

  updateCommitPoint(id: string, changes: { mode?: "auto" | "approve" }): CommitPoint[] {
    return this.batchQueue.updateCommitPoint(id, changes);
  }

  /** IPC-exposed: run the git commit for a commit point immediately. */
  commitCommitPoint(id: string, message: string): CommitPoint {
    return this.batchQueue.executeCommit(id, message);
  }

  /**
   * Core "commit now" helper shared by the auto-commit-mode path (no DB row)
   * and the manually-placed auto-mode commit point path (row exists, caller
   * flips it to done). Generates a message from settled work items, runs
   * `git commit`, returns sha+message on success or null on any failure. All
   * errors are logged here so callers can stay simple.
   */
  private runAutoCommit(batch: Batch, workItems: WorkItem[]): { sha: string; message: string } | null {
    const stream = this.store.get(batch.stream_id);
    if (!stream) {
      this.logger.warn("auto-commit: stream not found", { batchId: batch.id });
      return null;
    }
    const message = buildAutoCommitMessage(workItems);
    try {
      const result = gitCommitAll(stream.worktree_path, message, { includeUntracked: true });
      if (!result.ok || !result.sha) {
        this.logger.warn("auto-commit: git commit failed", {
          batchId: batch.id,
          stderr: result.stderr,
        });
        return null;
      }
      this.logger.info("auto-commit: committed", { batchId: batch.id, sha: result.sha, message });
      return { sha: result.sha, message };
    } catch (err) {
      this.logger.warn("auto-commit: execution error", {
        batchId: batch.id,
        error: err instanceof Error ? err.message : String(err),
      });
      return null;
    }
  }

  /**
   * Execute an auto-mode commit point: run the shared `runAutoCommit`, then
   * mark the commit point done so decideStopDirective sees it as terminal.
   * Used for manually-placed commit points with mode="auto". The no-row
   * auto-commit-mode path calls `runAutoCommit` directly.
   */
  private executeAutoCommitPoint(cp: CommitPoint, batch: Batch, workItems: WorkItem[]): void {
    const result = this.runAutoCommit(batch, workItems);
    if (!result) return;
    this.commitPointStore.markCommitted(cp.id, result.message, result.sha);
  }

  reorderBatchQueue(
    streamId: string,
    batchId: string,
    entries: Array<{ kind: "work" | "commit" | "wait"; id: string }>,
  ): void {
    this.resolveBatch(streamId, batchId);
    this.batchQueue.reorderBatchQueue(batchId, entries);
  }

  // -------- wait points (IPC-exposed delegations) --------

  listWaitPoints(batchId: string): WaitPoint[] {
    return this.batchQueue.listWaitPoints(batchId);
  }

  createWaitPoint(streamId: string, batchId: string, note?: string | null): WaitPoint {
    this.resolveBatch(streamId, batchId);
    return this.batchQueue.createWaitPoint(batchId, note ?? null);
  }

  setWaitPointNote(id: string, note: string | null): WaitPoint {
    return this.batchQueue.setWaitPointNote(id, note);
  }

  deleteWaitPoint(id: string): void {
    this.batchQueue.deleteWaitPoint(id);
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

export function buildNextWorkItemStopReason(
  item: { id: string; title: string; kind: string; batch_id: string },
  _streamId: string,
  context: { uiChangeNudge?: boolean } = {},
): string {
  const lines: string[] = [];
  if (context.uiChangeNudge) {
    lines.push(
      `⚠ UI change detected in this turn (src/ui/** paths). Restart newde and exercise the feature in the browser before the subagent marks any item human_check; say so explicitly in your work-item note if you couldn't visually verify.`,
      ``,
    );
  }
  lines.push(
    `The batch queue has ready work (batchId="${item.batch_id}"). Call \`mcp__newde__read_work_options\` and dispatch to a \`general-purpose\` subagent per the newde-task-dispatch skill.`,
  );
  return lines.join("\n");
}

/**
 * Find the lowest-sort_index commit point with mode="auto" and status="pending"
 * whose preceding work items are all terminal. Returns null if none qualifies.
 * Mirrors the logic in `decideStopDirective`'s findActiveMarker helper.
 */
function findActiveAutoCommitPoint(
  commitPoints: CommitPoint[],
  workItems: WorkItem[],
): CommitPoint | null {
  // Sort ascending by sort_index so we pick the first eligible one.
  const sorted = [...commitPoints].sort((a, b) => a.sort_index - b.sort_index);
  for (const cp of sorted) {
    if (cp.status !== "pending" || cp.mode !== "auto") continue;
    const preceding = workItems.filter((item) => item.sort_index < cp.sort_index);
    const allTerminal = preceding.every(
      (item) =>
        item.status === "done" ||
        item.status === "canceled" ||
        item.status === "archived" ||
        item.status === "human_check",
    );
    if (allTerminal) return cp;
  }
  return null;
}

/**
 * Build an auto-generated commit message from settled work items. Uses the
 * titles of all human_check/done/canceled work items in the batch as the
 * body of a "chore:" conventional commit.
 */
export function buildAutoCommitMessage(workItems: WorkItem[]): string {
  const settled = workItems.filter(
    (item) =>
      item.status === "human_check" ||
      item.status === "done" ||
      item.status === "canceled",
  );
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

export function buildHumanCheckNudgeStopReason(item: WorkItem): string {
  const lines = [
    `You have one work item still \`in_progress\` but you didn't update it during this turn: "${item.title}" (id=${item.id}).`,
    ``,
    `If its acceptance criteria are met, call \`mcp__newde__update_work_item\` with \`status: "human_check"\` — don't leave finished work parked in IN PROGRESS.`,
    ``,
    `If the work isn't done, either: (a) call \`mcp__newde__add_work_note\` summarizing what's still needed (this suppresses the nudge), or (b) call \`update_work_item\` with \`status: "blocked"\` if you're stuck and a user decision is required.`,
  ];
  return lines.join("\n");
}

export function buildCommitPointStopReason(cp: CommitPoint): string {
  const lines = [
    `A commit point is due in this batch's work queue (commit_point_id=${cp.id}).`,
    ``,
    `Inspect the unstaged/staged changes since the last commit using read-only commands (\`git status\`, \`git diff\`, \`git diff --staged\`), then draft a concise commit message describing those changes. Keep the subject terse and descriptive — avoid Conventional-Commits prefixes like \`feat(scope):\` or \`fix:\`. Do NOT add Co-Authored-By or any self-attribution lines.`,
    ``,
    `Output the drafted message in your chat reply and ask the user to approve or suggest changes. Do NOT run \`git add\` or \`git commit\` yourself.`,
    ``,
    `When the user approves, call \`mcp__newde__commit\` with { commit_point_id: "${cp.id}", message: "<final message>" } — that runs the git commit. If the user suggests changes, redraft the message in your next reply and ask again; only call \`mcp__newde__commit\` once the user has explicitly approved.`,
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

function buildBatchAgentPrompt(
  stream: Stream,
  batch: Batch,
  agentPromptAppend: string,
  activeBatch?: Batch | null,
): string {
  // Keep this preamble SITUATIONAL only — procedural "how to use the work-item
  // tools" policy lives in the `newde-task-filing` / `newde-task-lifecycle` /
  // `newde-task-dispatch` skills so it's only loaded when the agent actually
  // needs the relevant slice. Every line here is replayed via cache-read on
  // every turn; treat additions as expensive.
  const lines = [
    `SESSION CONTEXT: stream "${stream.title}" (id: ${stream.id}), batch "${batch.title}" (id: ${batch.id}). Pass batchId="${batch.id}" to all newde work-item tools.`,
    activeBatch && activeBatch.id !== batch.id
      ? `ACTIVE (writer) batch: "${activeBatch.title}" (id: ${activeBatch.id}). Only that batch can commit; your batch is read-only.`
      : `Your batch is the ACTIVE writer — the only batch allowed to commit.`,
    `Start each session by calling \`newde__read_work_options\`; the newde-task-filing, newde-task-lifecycle, and newde-task-dispatch skills cover the orchestrator/subagent pattern, filing conventions, status transitions, and how to reference items in user-facing text.`,
  ];
  if (batch.status !== "active") {
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
  const batchPrompt = batch.custom_prompt?.trim();
  if (batchPrompt) {
    lines.push("", "# Batch instructions", "", batchPrompt);
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
  batch: { id: string; title: string };
  activeBatch: { id: string; title: string } | null;
  /**
   * The batch's writer/read-only role at the moment this Claude session
   * was first seen. When set and different from the *current* role, a
   * loud ROLE CHANGE banner is appended before `</session-context>` to
   * supersede the (frozen, cache-read) NON_WRITER_PROMPT_BLOCK in the
   * initial system prompt. Omitting the field is a no-op — older call
   * sites keep the original single-line writer: rendering.
   */
  initialRole?: "writer" | "read-only";
}): string {
  const { stream, batch, activeBatch, initialRole } = input;
  const currentRole: "writer" | "read-only" =
    activeBatch && activeBatch.id !== batch.id ? "read-only" : "writer";
  const lines = [
    `<session-context>`,
    `stream: "${stream.title}" (id: ${stream.id})`,
    `batch:  "${batch.title}" (id: ${batch.id})`,
    activeBatch && activeBatch.id !== batch.id
      ? `writer: "${activeBatch.title}" (id: ${activeBatch.id}) — your batch is read-only`
      : `writer: (you) — your batch is the active writer`,
  ];
  if (initialRole && initialRole !== currentRole) {
    if (currentRole === "writer") {
      lines.push(
        "ROLE CHANGE: this batch was read-only when the session started; it is now the active writer. The NON_WRITER block in your initial system prompt is SUPERSEDED — you may now use Write/Edit/Bash to mutate the worktree.",
      );
    } else {
      lines.push(
        "ROLE CHANGE: this batch was the active writer when the session started; it is now read-only. The NON_WRITER block applies now even though it wasn't in your initial system prompt — Write/Edit/Bash mutations to the worktree will be blocked.",
      );
    }
  }
  lines.push(`</session-context>`);
  return lines.join("\n");
}

export function buildBatchMcpConfig(mcp: McpServerHandle | null): string {
  if (!mcp) {
    throw new Error("mcp server not started");
  }
  return JSON.stringify({
    mcpServers: {
      newde: {
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

function streamWorktreePath(projectDir: string, branch: string): string {
  return join(projectDir, ".newde", "worktrees", sanitizeBranch(branch));
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
  turnStore: TurnStore;
  flushSnapshot: (source: SnapshotSource) => string | null;
}

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
    batchId: string;
    workItemId: string;
    previous: WorkItem["status"] | undefined;
    next: WorkItem["status"] | undefined;
    touchedFiles?: string[];
  },
): void {
  const { previous, next, batchId, workItemId, touchedFiles } = params;
  if (!next) return;
  if (next === "in_progress" && previous !== "in_progress") {
    const startSnapshotId = deps.flushSnapshot("task-start");
    const effort = deps.effortStore.openEffort({ workItemId, startSnapshotId });
    const openTurn = deps.turnStore.currentOpenTurn(batchId);
    if (openTurn) deps.effortStore.linkEffortTurn(effort.id, openTurn.id);
  } else if (previous === "in_progress" && next !== "in_progress") {
    // Capture the effort id *before* closing — closeEffort clears the
    // "open effort" marker. We need the id to attach the touched-files
    // payload to the row just closed.
    const openEffort = deps.effortStore.getOpenEffort(workItemId);
    const endSnapshotId = deps.flushSnapshot("task-end");
    deps.effortStore.closeEffort({ workItemId, endSnapshotId });
    if (openEffort && next === "human_check" && Array.isArray(touchedFiles) && touchedFiles.length > 0) {
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

/** Attach every currently-open effort to `turnId`. No-op when there are none. */
export function linkOpenEffortsToTurn(effortStore: WorkItemEffortStore, turnId: string): void {
  for (const effort of effortStore.listOpenEfforts()) {
    effortStore.linkEffortTurn(effort.id, turnId);
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

function derivePostToolStatus(resp: unknown): "ok" | "error" {
  if (!resp || typeof resp !== "object") return "ok";
  const obj = resp as Record<string, unknown>;
  if (obj.error != null && obj.error !== "") return "error";
  if (obj.is_error === true) return "error";
  return "ok";
}
