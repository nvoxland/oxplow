import { EventEmitter } from "node:events";
import { existsSync, watch, type FSWatcher } from "node:fs";
import { join, resolve } from "node:path";
import { randomUUID } from "node:crypto";
import { buildAgentCommandForSession } from "../agent/agent-command.js";
import { buildWriteGuardResponse, NON_WRITER_PROMPT_BLOCK } from "./write-guard.js";
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
import { ResumeTracker } from "../session/resume-tracker.js";
import { createElectronPlugin, type ElectronPlugin } from "../session/claude-plugin.js";
import { startMcpServer, type HookEnvelope, type McpServerHandle } from "../mcp/mcp-server.js";
import { buildWorkItemMcpTools } from "../mcp/mcp-tools.js";
import { buildLspMcpTools } from "../mcp/lsp-mcp-tools.js";
import { getStateDatabase } from "../persistence/state-db.js";
import { StreamStore, type PaneKind, type Stream } from "../persistence/stream-store.js";
import { BACKLOG_SCOPE, WorkItemStore } from "../persistence/work-item-store.js";
import { CommitPointStore, type CommitPoint, type CommitPointMode } from "../persistence/commit-point-store.js";
import { WaitPointStore, type WaitPoint } from "../persistence/wait-point-store.js";
import { TurnStore, type AgentTurn } from "../persistence/turn-store.js";
import {
  FileChangeStore,
  type BatchFileChange,
  type FileChangeKind,
  type FileChangeSource,
} from "../persistence/file-change-store.js";
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
  readonly turnStore: TurnStore;
  readonly fileChangeStore: FileChangeStore;
  readonly workItemApi: WorkItemApi;
  readonly hookEvents: HookEventStore;
  readonly resumeTracker: ResumeTracker;
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
  private mcp: McpServerHandle | null = null;
  private gitEnabledCached = false;
  private gitRootWatcher: FSWatcher | null = null;

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
    this.turnStore = new TurnStore(projectDir, logger.child({ subsystem: "turn-store" }));
    this.fileChangeStore = new FileChangeStore(projectDir, logger.child({ subsystem: "file-change-store" }));
    this.workItemApi = createWorkItemApi({
      resolveBatch: (streamId, batchId) => this.resolveBatch(streamId, batchId),
      workItemStore: this.workItemStore,
      turnStore: this.turnStore,
      fileChangeStore: this.fileChangeStore,
    });
    this.events = new EventBus(logger.child({ subsystem: "event-bus" }));
    this.hookEvents = new HookEventStore(1000);
    this.resumeTracker = new ResumeTracker();
    this.lspManager = new LspSessionManager(logger.child({ subsystem: "lsp" }));
    this.editorFocusStore = new EditorFocusStore();
    this.agentPtyStore = new AgentPtyStore();
    this.workspaceWatchers = new WorkspaceWatcherRegistry(logger.child({ subsystem: "workspace-watch" }));
    this.gitRefsWatchers = new GitRefsWatcherRegistry(logger.child({ subsystem: "git-refs-watch" }));
  }

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

    for (const existingStream of this.store.list()) {
      this.batchStore.ensureStream(existingStream);
      this.workspaceWatchers.ensureWatching(existingStream);
      this.gitRefsWatchers.ensureWatching(existingStream);
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
      // Approved proposals get committed eagerly on the runtime side so the
      // agent sees the queue move forward without further prompting.
      if (change.kind === "updated" && change.id) {
        const cp = this.commitPointStore.get(change.id);
        if (cp?.status === "approved") this.executeApprovedCommit(cp);
      }
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

    // Crash recovery: any commit points left in `approved` state from a prior
    // run haven't had their `git commit` executed yet. Drain them now.
    for (const cp of this.commitPointStore.listApproved()) {
      this.executeApprovedCommit(cp);
    }

    this.mcp = await startMcpServer({
      workspaceFolders: this.store.list().map((candidate) => candidate.worktree_path),
      logger: this.logger.child({ subsystem: "mcp" }),
      extraTools: [
        ...buildWorkItemMcpTools({
          resolveStream: (streamId) => this.resolveStream(streamId),
          resolveBatch: (streamId, batchId) => this.resolveBatch(streamId, batchId),
          batchStore: this.batchStore,
          workItemStore: this.workItemStore,
          commitPointStore: this.commitPointStore,
          turnStore: this.turnStore,
          fileChangeStore: this.fileChangeStore,
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
    this.gitRootWatcher?.close();
    this.gitRootWatcher = null;
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
      const created = ensureAgentPane(
        batch.pane_target,
        stream.worktree_path,
        cols,
        rows,
        agentCommand,
        { signatureSource, logger: paneLogger },
      );
      if (created) {
        this.resumeTracker.noteSessionLaunch(`${stream.id}:${batch.id}`, !!batch.resume_session_id);
      }
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
      const alreadySpawned = this.agentPtyStore.get(batch.id) !== null;
      const agentPty = this.agentPtyStore.ensure(
        batch.id,
        { command: agentCommand, cwd: stream.worktree_path, cols, rows },
        paneLogger.child({ subsystem: "agent-pty" }),
      );
      if (!alreadySpawned) {
        this.resumeTracker.noteSessionLaunch(`${stream.id}:${batch.id}`, !!batch.resume_session_id);
      }
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
          appendSystemPrompt: buildBatchAgentPrompt(stream, batch, this.config.agentPromptAppend),
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
    const pane: PaneKind | undefined = envelope.pane === "working" || envelope.pane === "talking"
      ? envelope.pane
      : undefined;
    const stored = ingestHookPayload(this.hookEvents, envelope.event, envelope.payload, {
      streamId,
      batchId: envelope.batchId,
      pane,
    });
    if (envelope.batchId && this.store.get(streamId)) {
      const update = this.resumeTracker.recordSessionHookEvent(
        `${streamId}:${envelope.batchId}`,
        envelope.event,
        stored.normalized.sessionId,
      );
      if (update?.type === "set") {
        this.batchStore.updateResume(streamId, envelope.batchId, update.sessionId);
      } else if (update?.type === "clear") {
        this.batchStore.updateResume(streamId, envelope.batchId, "");
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
      const additionalContext = formatEditorFocusForAgent(this.editorFocusStore.get(streamId));
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
      if (directive) return { body: directive };
    }
  }

  /**
   * Stop-hook pipeline. Runs in priority order:
   *   1. Pending commit point whose prior items are all terminal: block and
   *      tell the agent to propose a commit message.
   *   2. Pending wait point whose prior items are terminal: flip to
   *      `triggered` and let the agent stop; the user clicks Continue.
   *   3. Approval-mode commit point sitting at `proposed`: let the agent stop
   *      while the user reviews the message.
   *   4. No blockers but a ready work item exists and this is the writer
   *      batch: block and tell the agent to pick it up so the queue drains
   *      without manual re-prompting.
   * Returns the hook body or null to allow Stop.
   */
  private computeStopDirective(batchId: string): Record<string, unknown> | null {
    const activeCommit = this.findActiveCommitPoint(batchId);
    if (activeCommit && activeCommit.status === "pending") {
      return { decision: "block", reason: buildCommitPointStopReason(activeCommit) };
    }
    const activeWait = this.findActiveWaitPoint(batchId);
    if (activeWait) {
      // Flip to triggered so the UI can surface "agent stopped here" and let
      // the agent stop. The user re-engages by prompting the agent directly;
      // findActiveWaitPoint skips triggered points so the next Stop resumes
      // auto-progression past this marker.
      try { this.waitPointStore.trigger(activeWait.id); } catch {}
      return null;
    }
    if (activeCommit && activeCommit.status === "proposed") {
      // Approval-mode commit awaiting user review — let the agent rest.
      return null;
    }
    const batch = this.batchStore.findById(batchId);
    if (!batch || batch.status !== "active") return null;
    const ready = this.workItemStore.listReady(batchId);
    if (ready.length === 0) return null;
    const next = ready[0]!;
    return {
      decision: "block",
      reason: buildNextWorkItemStopReason(next.id, next.title, next.kind),
    };
  }

  /** Lowest-sort_index non-done wait point whose preceding work items are
   *  all terminal. Mirrors findActiveCommitPoint. */
  findActiveWaitPoint(batchId: string): WaitPoint | null {
    const points = this.waitPointStore.listForBatch(batchId);
    const workItems = this.workItemStore.listItems(batchId);
    for (const wp of points) {
      // `triggered` points are "consumed" — they already stopped the agent
      // once and the user has re-engaged; don't stop again for them.
      if (wp.status !== "pending") continue;
      const preceding = workItems.filter((item) => item.sort_index < wp.sort_index);
      const allTerminal = preceding.every((item) => item.status === "done" || item.status === "canceled");
      if (!allTerminal) continue;
      return wp;
    }
    return null;
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
        this.turnStore.openTurn({ batchId, prompt, sessionId });
        return;
      }
      case "Stop": {
        const open = this.turnStore.currentOpenTurn(batchId);
        if (!open) return;
        const batch = this.batchStore.findById(batchId);
        const answer = batch?.summary?.trim() ? batch.summary : null;
        const workItemId = this.soleInProgressWorkItem(batchId);
        this.turnStore.closeTurn(open.id, { workItemId, answer });
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

  // -------- commit points (IPC-exposed methods) --------

  listCommitPoints(batchId: string): CommitPoint[] {
    return this.commitPointStore.listForBatch(batchId);
  }

  createCommitPoint(streamId: string, batchId: string, mode: CommitPointMode): CommitPoint {
    this.resolveBatch(streamId, batchId);
    // A commit point with no preceding work items has nothing to commit; refuse
    // to create one as the very first queue entry. The mixed reorder still
    // lets users drag a point above all work items if they really insist.
    const hasWork = this.workItemStore.listItems(batchId).length > 0;
    if (!hasWork) {
      throw new Error("cannot add a commit point before any work items exist");
    }
    const sortIndex = this.nextQueueSortIndex(batchId);
    return this.commitPointStore.create({ batchId, mode, sortIndex });
  }

  setCommitPointMode(id: string, mode: CommitPointMode): CommitPoint {
    return this.commitPointStore.setMode(id, mode);
  }

  approveCommitPoint(id: string, editedMessage?: string): CommitPoint {
    return this.commitPointStore.approve(id, editedMessage);
  }

  rejectCommitPoint(id: string, note: string): CommitPoint {
    return this.commitPointStore.reject(id, note);
  }

  resetCommitPoint(id: string): CommitPoint {
    return this.commitPointStore.resetToPending(id);
  }

  deleteCommitPoint(id: string): void {
    this.commitPointStore.delete(id);
  }

  /**
   * Reorder the mixed batch queue (work items + commit points). `entries` is
   * the desired top-to-bottom order; sort_indexes are rewritten to match so
   * findActiveCommitPoint and the Stop-hook pipeline see the new position.
   */
  reorderBatchQueue(
    streamId: string,
    batchId: string,
    entries: Array<{ kind: "work" | "commit" | "wait"; id: string }>,
  ): void {
    this.resolveBatch(streamId, batchId);
    const workEntries: Array<{ id: string; sortIndex: number }> = [];
    const commitEntries: Array<{ id: string; sortIndex: number }> = [];
    const waitEntries: Array<{ id: string; sortIndex: number }> = [];
    entries.forEach((entry, index) => {
      if (entry.kind === "work") workEntries.push({ id: entry.id, sortIndex: index });
      else if (entry.kind === "commit") commitEntries.push({ id: entry.id, sortIndex: index });
      else waitEntries.push({ id: entry.id, sortIndex: index });
    });
    this.workItemStore.setItemSortIndexes(batchId, workEntries);
    this.commitPointStore.setSortIndexes(commitEntries);
    this.waitPointStore.setSortIndexes(waitEntries);
  }

  // -------- wait points (IPC-exposed methods) --------

  listWaitPoints(batchId: string): WaitPoint[] {
    return this.waitPointStore.listForBatch(batchId);
  }

  createWaitPoint(streamId: string, batchId: string, note?: string | null): WaitPoint {
    this.resolveBatch(streamId, batchId);
    // Wait points have nothing to "wait after" without preceding work items;
    // refuse to create one as the very first queue entry.
    const hasWork = this.workItemStore.listItems(batchId).length > 0;
    if (!hasWork) {
      throw new Error("cannot add a wait point before any work items exist");
    }
    const sortIndex = this.nextQueueSortIndex(batchId);
    return this.waitPointStore.create({ batchId, sortIndex, note: note ?? null });
  }

  setWaitPointNote(id: string, note: string | null): WaitPoint {
    return this.waitPointStore.setNote(id, note);
  }

  deleteWaitPoint(id: string): void {
    this.waitPointStore.delete(id);
  }

  /**
   * The next commit point the agent should act on for a batch — the
   * lowest-sort_index non-done/non-rejected point whose preceding work items
   * are all done or canceled. Returns null if nothing is due.
   */
  findActiveCommitPoint(batchId: string): CommitPoint | null {
    const points = this.commitPointStore.listForBatch(batchId);
    const workItems = this.workItemStore.listItems(batchId);
    for (const cp of points) {
      if (cp.status === "done") continue;
      // A point is "ready" only if every non-commit item with a smaller
      // sort_index has reached a terminal state. `sort_index` on commit_point
      // lives in the same numeric space as work_items via nextQueueSortIndex.
      const preceding = workItems.filter((item) => item.sort_index < cp.sort_index);
      const allTerminal = preceding.every((item) => item.status === "done" || item.status === "canceled");
      if (!allTerminal) continue;
      return cp;
    }
    return null;
  }

  private nextQueueSortIndex(batchId: string): number {
    const items = this.workItemStore.listItems(batchId);
    const commits = this.commitPointStore.listForBatch(batchId);
    const waits = this.waitPointStore.listForBatch(batchId);
    const maxItem = items.reduce((m, item) => Math.max(m, item.sort_index), -1);
    const maxCommit = commits.reduce((m, p) => Math.max(m, p.sort_index), -1);
    const maxWait = waits.reduce((m, p) => Math.max(m, p.sort_index), -1);
    return Math.max(maxItem, maxCommit, maxWait) + 1;
  }

  private executeApprovedCommit(cp: CommitPoint): void {
    // Resolve the stream via the batch. If the batch was deleted or the
    // worktree is gone, mark the point rejected with a note rather than
    // looping forever.
    const batch = this.batchStore.findById(cp.batch_id);
    if (!batch) {
      this.logger.warn("commit point has no batch; dropping", { id: cp.id });
      return;
    }
    const stream = this.store.get(batch.stream_id);
    if (!stream) {
      this.logger.warn("commit point has no stream; dropping", { id: cp.id });
      return;
    }
    const message = cp.approved_message ?? cp.proposed_message;
    if (!message) {
      this.logger.warn("commit point approved with no message; skipping", { id: cp.id });
      return;
    }
    const result = gitCommitAll(stream.worktree_path, message);
    if (!result.ok || !result.sha) {
      this.logger.warn("git commit failed for commit point", {
        id: cp.id,
        stderr: result.stderr,
      });
      // Move out of `approved` to `rejected` so the startup-recovery loop
      // doesn't retry forever. The user can read the rejection_note in the
      // UI and click Retry to send the point back through the agent.
      this.commitPointStore.failExecution(cp.id, `commit failed: ${result.stderr || "unknown"}`);
      return;
    }
    try {
      this.commitPointStore.markDone(cp.id, result.sha);
      this.logger.info("committed for commit point", { id: cp.id, sha: result.sha });
    } catch (err) {
      this.logger.warn("failed to mark commit point done", {
        id: cp.id,
        error: err instanceof Error ? err.message : String(err),
      });
    }
  }
}

export function buildNextWorkItemStopReason(itemId: string, title: string, kind: string): string {
  return [
    `The current work item is done but the batch queue still has ready work. Pick up this item and continue:`,
    ``,
    `- work_item_id: ${itemId}`,
    `- kind: ${kind}`,
    `- title: ${title}`,
    ``,
    `Mark it in_progress via \`mcp__newde__update_work_item\`, execute the work, then mark it done before stopping again. Do not stop until the queue is empty, a commit point is due, or a wait point is hit.`,
  ].join("\n");
}

export function buildCommitPointStopReason(cp: CommitPoint): string {
  const lines = [
    `A commit point is due in this batch's work queue (commit_point_id=${cp.id}, mode=${cp.mode}).`,
    ``,
    `Inspect the unstaged/staged changes since the last commit (use read-only commands like \`git status\`, \`git diff\`, \`git diff --staged\`), then draft a concise commit message in Conventional-Commits style describing those changes.`,
    ``,
    `Call \`mcp__newde__propose_commit\` with { commit_point_id: "${cp.id}", message: "<your message>" }. Do NOT run \`git add\` or \`git commit\` yourself — the runtime executes the commit once the message is approved.`,
  ];
  if (cp.mode === "auto") {
    lines.push(``, `Mode is "auto": the runtime will commit immediately after you propose, so your message is final.`);
  } else {
    lines.push(``, `Mode is "approval": after you propose, wait — the user will approve, edit, or reject the message. You'll see the outcome on your next turn.`);
  }
  if (cp.rejection_note) {
    lines.push(``, `Previous attempt was rejected with this note: ${cp.rejection_note}`);
  }
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

function buildBatchAgentPrompt(stream: Stream, batch: Batch, agentPromptAppend: string): string {
  const lines = [
    `You manage this batch's work through the newde work-item MCP tools.`,
    `Treat them as your durable working memory between turns and sessions; the human watches progress through them.`,
    ``,
    `PLAN BEFORE CODING. For anything that will take more than about two minutes, call newde__create_work_item before starting.`,
    `Use the title for intent, description for the approach, and acceptanceCriteria for a checklist of observable conditions that define "done" (plain text, one per line).`,
    `Set parentId when the item rolls up under a larger epic or task.`,
    ``,
    `DECOMPOSE DELIBERATELY: epic = multi-step feature, task = concrete unit of work, subtask = small step inside a task, bug = defect to fix, note = observation that doesn't need execution.`,
    ``,
    `WORK THE READY QUEUE. Start each turn with newde__list_ready_work to pick the highest-priority unblocked item.`,
    `Set its status to "in_progress" via newde__update_work_item before touching code.`,
    `Post newde__add_work_note at meaningful milestones.`,
    `The moment every acceptance criterion is met, set status to "done".`,
    `Use newde__get_work_item when resuming a specific item — its response includes links and recent audit events so you can pick up where you left off.`,
    ``,
    `LINK DEPENDENCIES with newde__link_work_items. linkType is one of: "blocks" (from-item must finish before to-item can start), "discovered_from" (file newly-uncovered work as its own item linked back to the item you were on — do NOT scope-creep the original), "relates_to" (general association), "duplicates" (same work as an existing item; delete or supersede the duplicate), "supersedes" (replaces a stale older item), "replies_to" (threaded response to another item).`,
    ``,
    `STAY HONEST. Rewrite description / acceptanceCriteria when your understanding shifts. newde__delete_work_item anything you've decided against instead of letting it rot in "waiting".`,
    ``,
    `BEFORE ENDING EACH TURN, call newde__record_batch_summary with a 2-3 sentence description of what has been happening in this batch overall and what changed in the latest round. Rewrite from scratch so it reflects the current state of the work-item log.`,
    ``,
    `SESSION CONTEXT: stream "${stream.title}" (id: ${stream.id}), batch "${batch.title}" (id: ${batch.id}).`,
    `Always pass batchId="${batch.id}" to every work-item tool.`,
    `Call newde__get_batch_context whenever you need to re-check stream/batch ids or read the current batch summary.`,
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
