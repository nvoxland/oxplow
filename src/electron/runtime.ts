import { EventEmitter } from "node:events";
import { existsSync, readdirSync, statSync, watch, type FSWatcher } from "node:fs";
import { join, resolve } from "node:path";
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
  type GitLogResult,
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
import { BACKLOG_SCOPE, WorkItemStore } from "../persistence/work-item-store.js";
import { CommitPointStore, type CommitPoint } from "../persistence/commit-point-store.js";
import { WaitPointStore, type WaitPoint } from "../persistence/wait-point-store.js";
import { TurnStore, type AgentTurn } from "../persistence/turn-store.js";
import {
  FileChangeStore,
  type BatchFileChange,
  type FileChangeKind,
  type FileChangeSource,
} from "../persistence/file-change-store.js";
import {
  SnapshotStore,
  type FileSnapshot,
  type SnapshotDiffResult,
  type SnapshotKind,
  type SnapshotSummary,
} from "../persistence/snapshot-store.js";
import { shouldIgnoreWorkspaceWatchPath } from "../git/workspace-watch.js";
import { createWorkItemApi, type WorkItemApi } from "./work-item-api.js";
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
  readonly fileChangeStore: FileChangeStore;
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
    this.fileChangeStore = new FileChangeStore(projectDir, logger.child({ subsystem: "file-change-store" }));
    this.snapshotStore = new SnapshotStore(projectDir, logger.child({ subsystem: "snapshot-store" }));
    this.snapshotStore.setMaxFileBytes(config.snapshotMaxFileBytes);
    this.workItemApi = createWorkItemApi({
      resolveBatch: (streamId, batchId) => this.resolveBatch(streamId, batchId),
      workItemStore: this.workItemStore,
      turnStore: this.turnStore,
      fileChangeStore: this.fileChangeStore,
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
      this.seedSnapshotTracking(existingStream.id);
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
      this.recordFsWatchChange(event.streamId, event.path, event.kind, event.t);
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
    this.fileChangeStore.subscribe((change) => {
      const batch = this.batchStore.findById(change.batch_id);
      if (!batch) return;
      this.events.publish({
        type: "file-change.recorded",
        streamId: batch.stream_id,
        batchId: change.batch_id,
        turnId: change.turn_id,
        changeId: change.id,
        path: change.path,
        kind: change.change_kind,
        source: change.source,
      });
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
          fileChangeStore: this.fileChangeStore,
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
  private buildRefreshedSessionContext(envelopeBatchId: string | null, streamId: string): string {
    void streamId;
    const batch = envelopeBatchId ? this.batchStore.findById(envelopeBatchId) : null;
    if (!batch) return "";
    const stream = this.store.get(batch.stream_id);
    if (!stream) return "";
    const batchState = this.batchStore.list(stream.id);
    const activeBatch = batchState.batches.find((b) => b.id === batchState.activeBatchId) ?? null;
    return buildSessionContextBlock({ stream, batch, activeBatch });
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
      const deny = buildWriteGuardResponse(batch, toolName);
      if (deny) return { body: deny };
    }
    if (envelope.event === "UserPromptSubmit") {
      const focusContext = formatEditorFocusForAgent(this.editorFocusStore.get(streamId));
      // Re-inject the session context each turn — the agent's system-prompt
      // SESSION CONTEXT line is frozen at launch, but the UI's active /
      // selected batch can flip mid-session. Reading the live state here
      // keeps the agent pointed at the right ids without a user-visible
      // prompt edit.
      const sessionContext = this.config.injectSessionContext
        ? this.buildRefreshedSessionContext(envelope.batchId ?? null, streamId)
        : "";
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
    const currentTurnFilePaths = openTurn
      ? this.fileChangeStore.listForTurn(openTurn.id).map((row) => row.path)
      : [];
    const snapshot: BatchSnapshot = {
      batch,
      commitPoints: this.commitPointStore.listForBatch(batchId),
      waitPoints: this.waitPointStore.listForBatch(batchId),
      workItems: this.workItemStore.listItems(batchId),
      readyWorkItems: this.workItemStore.listReady(batchId),
      currentTurnStartedAt: openTurn?.started_at ?? null,
      currentTurnFilePaths,
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
        // Defensive: if a prior turn never saw Stop, close it out with no answer
        // so every open turn corresponds to the latest prompt.
        const stillOpen = this.turnStore.currentOpenTurn(batchId);
        if (stillOpen) {
          this.turnStore.closeTurn(stillOpen.id, { workItemId: null, answer: null });
        }
        const turn = this.turnStore.openTurn({ batchId, prompt, sessionId });
        const batch = this.batchStore.findById(batchId);
        if (batch) {
          try {
            this.flushSnapshotForStream(batch.stream_id, "turn-start", turn.id, batchId);
          } catch (error) {
            this.logger.warn("turn-start snapshot flush failed", {
              batchId,
              turnId: turn.id,
              error: error instanceof Error ? error.message : String(error),
            });
          }
        }
        return;
      }
      case "Stop": {
        const open = this.turnStore.currentOpenTurn(batchId);
        if (!open) return;
        const batch = this.batchStore.findById(batchId);
        const workItemId = this.soleInProgressWorkItem(batchId);
        this.turnStore.closeTurn(open.id, { workItemId, answer: null });
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
          try {
            this.flushSnapshotForStream(batch.stream_id, "turn-end", open.id, batchId);
          } catch (error) {
            this.logger.warn("turn-end snapshot flush failed", {
              batchId,
              turnId: open.id,
              error: error instanceof Error ? error.message : String(error),
            });
          }
        }
        return;
      }
      case "SessionEnd": {
        const open = this.turnStore.currentOpenTurn(batchId);
        if (!open) return;
        this.turnStore.closeTurn(open.id, { workItemId: null, answer: null });
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
        const kind = this.classifyHookChangeKind(batchId, stream, normalizedPath, toolName);
        this.recordHookFileChange(batchId, normalizedPath, kind, toolName);
        return;
      }
      default:
        return;
    }
  }

  private recordFsWatchChange(streamId: string, path: string, kind: FileChangeKind, t: number): void {
    const stamp = this.recentUiWrites.get(path);
    if (stamp !== undefined && t - stamp < UI_WRITE_ECHO_WINDOW_MS) {
      return;
    }
    const activeBatchId = this.batchStore.list(streamId).activeBatchId;
    if (!activeBatchId) return;
    if (this.agentStatusByBatch.get(activeBatchId) !== "working") return;
    this.persistFileChange(activeBatchId, path, kind, "fs-watch", null);
  }

  private classifyHookChangeKind(
    batchId: string,
    stream: Stream | null,
    path: string,
    toolName: string,
  ): FileChangeKind {
    // Edit/MultiEdit/NotebookEdit require a pre-existing file by contract, so
    // the post-hook state is always "updated". Only Write can introduce a
    // brand-new file.
    if (toolName !== "Write") return "updated";
    // If we've already recorded a change for this path in this batch, it's no
    // longer the first write — classify as an update regardless of file state.
    if (this.fileChangeStore.hasChangeForPath(batchId, path)) return "updated";
    const worktreePath = stream?.worktree_path;
    if (!worktreePath) return "updated";
    // On PostToolUse the write has already landed, so a non-existent file here
    // would be unusual; default to "updated" in that case rather than lying
    // about a create.
    const exists = existsSync(resolve(worktreePath, path));
    return exists ? "created" : "updated";
  }

  private recordHookFileChange(
    batchId: string,
    path: string,
    kind: FileChangeKind,
    toolName: string,
  ): void {
    this.persistFileChange(batchId, path, kind, "hook", toolName);
  }

  private persistFileChange(
    batchId: string,
    path: string,
    kind: FileChangeKind,
    source: FileChangeSource,
    toolName: string | null,
  ): void {
    const turn = this.turnStore.currentOpenTurn(batchId);
    const workItemId = this.soleInProgressWorkItem(batchId);
    this.fileChangeStore.record({
      batchId,
      turnId: turn?.id ?? null,
      workItemId,
      path,
      changeKind: kind,
      source,
      toolName,
    });
    const batch = this.batchStore.findById(batchId);
    if (batch) this.markDirty(batch.stream_id, path);
  }

  private markDirty(streamId: string, path: string): void {
    let set = this.dirtyPathsByStream.get(streamId);
    if (!set) {
      set = new Set();
      this.dirtyPathsByStream.set(streamId, set);
    }
    set.add(path);
  }

  private flushSnapshotForStream(
    streamId: string,
    kind: SnapshotKind,
    turnId: string | null,
    batchId: string | null,
  ): string | null {
    const stream = this.store.get(streamId);
    if (!stream) return null;
    const dirty = this.dirtyPathsByStream.get(streamId);
    if (!dirty || dirty.size === 0) return null;
    const dirtyPaths = Array.from(dirty);
    const parent = this.store.getCurrentSnapshotId(streamId);
    const snapshotId = this.snapshotStore.flushSnapshot({
      kind,
      streamId,
      worktreePath: stream.worktree_path,
      dirtyPaths,
      parentSnapshotId: parent,
      turnId,
      batchId,
    });
    if (!snapshotId) return null;
    dirty.clear();
    this.store.setCurrentSnapshotId(streamId, snapshotId);
    // Backfill any pending batch_file_change rows for this stream that
    // haven't been attached to a snapshot yet.
    const placeholders = dirtyPaths.map(() => "?").join(",");
    getStateDatabase(this.projectDir).run(
      `UPDATE batch_file_change SET snapshot_id = ?
       WHERE snapshot_id IS NULL
         AND path IN (${placeholders})
         AND batch_id IN (SELECT id FROM batches WHERE stream_id = ?)`,
      snapshotId,
      ...dirtyPaths,
      streamId,
    );
    this.events.publish({
      type: "file-snapshot.created",
      streamId,
      snapshotId,
      kind,
      turnId,
      batchId,
    });
    return snapshotId;
  }

  /**
   * Walk the worktree in chunks to seed the dirty set. Two modes, unified
   * under one walker so large monorepos don't block the event loop either
   * way:
   *   - Cold start (no `current_snapshot_id`): mark every file dirty.
   *   - Reconcile (have a current snapshot): mark files whose disk stat
   *     differs from the resolved manifest entry, plus files that exist in
   *     the manifest but not on disk (deletions).
   *
   * Runs under `setImmediate` chunks of SEED_CHUNK_SIZE files; disposal
   * short-circuits further ticks via the `disposed` flag.
   */
  private seedSnapshotTracking(streamId: string): void {
    const stream = this.store.get(streamId);
    if (!stream) return;
    const ignore = (relpath: string) => shouldIgnoreWorkspaceWatchPath(relpath, this.config.generatedDirs);
    setImmediate(() => {
      if (this.disposed) return;
      try {
        const current = this.store.getCurrentSnapshotId(streamId);
        const entries = current ? this.snapshotStore.resolveEntries(current) : null;
        const seen = entries ? new Set<string>() : null;
        const stack: string[] = [""];
        let counter = 0;
        const step = () => {
          if (this.disposed) return;
          while (stack.length > 0 && counter < SEED_CHUNK_SIZE) {
            const rel = stack.pop()!;
            const abs = rel ? resolve(stream.worktree_path, rel) : stream.worktree_path;
            let children;
            try {
              children = readdirSync(abs, { withFileTypes: true });
            } catch {
              continue;
            }
            for (const child of children) {
              const childRel = rel ? `${rel}/${child.name}` : child.name;
              if (ignore(childRel)) continue;
              if (child.isDirectory()) {
                stack.push(childRel);
                continue;
              }
              if (!child.isFile()) continue;
              counter++;
              if (!entries) {
                this.markDirty(streamId, childRel);
                continue;
              }
              seen!.add(childRel);
              const entry = entries[childRel];
              if (!entry || entry.state === "deleted") {
                this.markDirty(streamId, childRel);
                continue;
              }
              let st;
              try {
                st = statSync(abs === stream.worktree_path ? resolve(stream.worktree_path, childRel) : resolve(abs, child.name));
              } catch {
                continue;
              }
              const size = st.size;
              const mtime = Math.floor(st.mtimeMs);
              if (entry.size !== size || entry.mtime_ms !== mtime) {
                this.markDirty(streamId, childRel);
              }
            }
          }
          if (stack.length > 0) {
            counter = 0;
            setImmediate(step);
            return;
          }
          // Walk finished. In reconcile mode, mark anything in the manifest
          // that disappeared from disk as dirty (the flush will record a
          // tombstone).
          if (entries && seen) {
            for (const [rel, entry] of Object.entries(entries)) {
              if (entry.state === "deleted") continue;
              if (!seen.has(rel)) this.markDirty(streamId, rel);
            }
          }
        };
        step();
      } catch (error) {
        this.logger.warn("seed snapshot tracking failed", {
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

  private soleInProgressWorkItem(batchId: string): string | null {
    const inProgress = this.workItemStore
      .listItems(batchId)
      .filter((item) => item.status === "in_progress");
    return inProgress.length === 1 ? inProgress[0]!.id : null;
  }

  listAgentTurns(batchId: string, limit?: number): AgentTurn[] {
    return this.turnStore.listForBatch(batchId, limit);
  }

  listFileChanges(batchId: string, limit?: number): BatchFileChange[] {
    return this.fileChangeStore.listForBatch(batchId, limit);
  }

  /**
   * Returns before/after contents for a single path within one turn.
   * "before" is the turn-start snapshot (or its parent if only an end
   * exists); "after" is the turn-end snapshot.
   */
  getTurnFileDiff(turnId: string, path: string): SnapshotDiffResult {
    const { start, end } = this.snapshotStore.getTurnSnapshots(turnId);
    if (!end) return { before: null, after: null, beforeState: "absent", afterState: "absent" };
    const beforeId = start?.parent_snapshot_id ?? end.parent_snapshot_id ?? null;
    return this.snapshotStore.diffPath(beforeId, end.id, path);
  }

  listSnapshots(streamId: string, limit?: number): FileSnapshot[] {
    return this.snapshotStore.listSnapshotsForStream(streamId, limit);
  }

  getSnapshotSummary(snapshotId: string): SnapshotSummary | null {
    return this.snapshotStore.getSnapshotSummary(snapshotId);
  }

  getSnapshotFileDiff(snapshotId: string, path: string): SnapshotDiffResult {
    return this.snapshotStore.getSnapshotFileDiff(snapshotId, path);
  }

  getSnapshotPairDiff(
    beforeSnapshotId: string | null,
    afterSnapshotId: string,
    path: string,
  ): SnapshotDiffResult {
    return this.snapshotStore.getSnapshotPairDiff(beforeSnapshotId, afterSnapshotId, path);
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
    `The batch queue has ready work. Dispatch it to a subagent:`,
    ``,
    `1. Call \`mcp__newde__read_work_options\` with batchId="${item.batch_id}" to get the next dispatch unit (may be an epic + its children, or a list of standalone items).`,
    `2. Launch a \`general-purpose\` subagent. Include in the brief:`,
    `   - The item ids, titles, descriptions, and acceptance criteria from the dispatch unit.`,
    `   - **REQUIRED — before touching any files:** call \`mcp__newde__update_work_item\` to mark each item \`in_progress\`. File-change attribution is driven by the sole in-progress item; if the subagent skips this step, changes will be attributed to the wrong item.`,
    `   - Mark \`human_check\` (not \`done\`) when acceptance criteria are met.`,
    `   - Use \`mcp__newde__add_work_note\` for decisions, surprises, or summaries.`,
    `   - Use \`mcp__newde__propose_commit\` when a commit point is due.`,
    `3. When the subagent returns, record a brief outcome note on each item via \`mcp__newde__add_work_note\`.`,
    `4. Repeat from step 1 until \`read_work_options\` returns \`{ mode: "empty" }\`, a commit point is due, or a wait point is hit.`,
  );
  return lines.join("\n");
}

export function buildCommitPointStopReason(cp: CommitPoint): string {
  const lines = [
    `A commit point is due in this batch's work queue (commit_point_id=${cp.id}).`,
    ``,
    `Inspect the unstaged/staged changes since the last commit (use read-only commands like \`git status\`, \`git diff\`, \`git diff --staged\`), then draft a concise commit message in Conventional-Commits style describing those changes.`,
    ``,
    `Call \`mcp__newde__propose_commit\` with { commit_point_id: "${cp.id}", message: "<your message>" } to record the draft, then output the message in your reply and ASK the user to approve. Do NOT run \`git add\` or \`git commit\` yourself.`,
    ``,
    `After the user replies: if they approve, call \`mcp__newde__commit\` with { commit_point_id: "${cp.id}", message: "<final message>" } — that runs the commit. If they suggest changes, call propose_commit again with the updated message and ask again.`,
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
  // tools" policy lives in the `newde-task-management` skill so it's only
  // loaded when the agent actually needs it. Every line here is replayed via
  // cache-read on every turn; treat additions as expensive.
  const lines = [
    `SESSION CONTEXT: stream "${stream.title}" (id: ${stream.id}), batch "${batch.title}" (id: ${batch.id}). Always pass batchId="${batch.id}" to newde work-item tools.`,
    activeBatch && activeBatch.id !== batch.id
      ? `ACTIVE (writer) batch: "${activeBatch.title}" (id: ${activeBatch.id}). Only that batch can commit; your batch is read-only.`
      : `Your batch is the ACTIVE writer — the only batch allowed to commit.`,
    `When referring to work items in text you show the user, use the item's TITLE (in quotes). Never print raw ids like "wi-abc123…" — the user sees titles in the UI, not ids.`,
    `See the \`newde-task-management\` skill for filing/managing work items; call \`newde__read_work_options\` at the start of a session to check for queued work and dispatch to a \`general-purpose\` subagent.`,
    `ORCHESTRATOR PATTERN: You are the orchestrator — never do Read/Edit/Bash/test work directly. For every work unit, call \`newde__read_work_options\` to get the dispatch unit, then launch one \`general-purpose\` subagent with all item ids, titles, descriptions, acceptance criteria, and instructions to write results back via MCP tools. Your context stays flat; all implementation detail lives in the subagent.`,
  ];
  if (batch.status !== "active") {
    lines.push(NON_WRITER_PROMPT_BLOCK);
  }
  const userAppend = agentPromptAppend.trim();
  if (userAppend) {
    lines.push("", "USER CUSTOM PROMPT:", userAppend);
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
}): string {
  const { stream, batch, activeBatch } = input;
  return [
    `<session-context>`,
    `stream: "${stream.title}" (id: ${stream.id})`,
    `batch:  "${batch.title}" (id: ${batch.id})`,
    activeBatch && activeBatch.id !== batch.id
      ? `writer: "${activeBatch.title}" (id: ${activeBatch.id}) — your batch is read-only`
      : `writer: (you) — your batch is the active writer`,
    `</session-context>`,
  ].join("\n");
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
// Yield back to the event loop after this many files during snapshot seed.
// Keeps large monorepos from blocking UI startup; exact value isn't critical.
const SEED_CHUNK_SIZE = 2000;

function extractEditedFilePath(input: unknown): string | null {
  if (!input || typeof input !== "object") return null;
  const obj = input as Record<string, unknown>;
  if (typeof obj.file_path === "string") return obj.file_path;
  if (typeof obj.notebook_path === "string") return obj.notebook_path;
  if (typeof obj.path === "string") return obj.path;
  return null;
}

function toWorktreeRelativePath(absOrRel: string, worktreePath: string): string {
  const normalizedRoot = resolve(worktreePath);
  const candidate = resolve(normalizedRoot, absOrRel);
  if (candidate === normalizedRoot) return "";
  if (candidate.startsWith(normalizedRoot + "/")) {
    return candidate.slice(normalizedRoot.length + 1);
  }
  return absOrRel;
}

function derivePostToolStatus(resp: unknown): "ok" | "error" {
  if (!resp || typeof resp !== "object") return "ok";
  const obj = resp as Record<string, unknown>;
  if (obj.error != null && obj.error !== "") return "error";
  if (obj.is_error === true) return "error";
  return "ok";
}
