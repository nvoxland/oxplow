import { EventEmitter } from "node:events";
import { mkdirSync, readFileSync, readdirSync, rmSync, watch, writeFileSync, type FSWatcher } from "node:fs";
import { join, resolve } from "node:path";
import { randomUUID } from "node:crypto";
import { buildAgentCommandForSession } from "../daemon.js";
import { ensureAgentPane } from "../fleet.js";
import { BatchStore, type Batch, type BatchState } from "../batch-store.js";
import { ensureWorktree, isGitRepo, listBranches, listGitStatuses } from "../git.js";
import { HookEventStore, ingestHookPayload, type StoredEvent } from "../hook-ingest.js";
import { LspSessionManager } from "../lsp.js";
import { createUiClientLogger, createDaemonLogger, type Logger, type LogLevel } from "../logger.js";
import { ResumeTracker } from "../resume-tracker.js";
import { createElectronSessionFiles, destroySessionFiles, type SessionFiles } from "../session-files.js";
import { startMcpServer, type McpServerHandle, type ToolDef } from "../mcp-server.js";
import { getStateDatabase } from "../state-db.js";
import { StreamStore, type PaneKind, type Stream } from "../stream-store.js";
import {
  WorkItemStore,
  type BatchWorkState,
  type WorkItemEvent,
  type WorkItemKind,
  type WorkItemPriority,
  type WorkItemStatus,
} from "../work-item-store.js";
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
} from "../workspace-files.js";
import { WorkspaceWatcherRegistry, type WorkspaceWatchEvent } from "../workspace-watch.js";
import { detectCurrentBranch } from "../git.js";
import { loadProjectConfig, type NewdeConfig } from "../config.js";
import { killSession } from "../tmux.js";
import { attachCommand, attachPane } from "../pty-bridge.js";
import type { UiLogPayload } from "./ipc-contract.js";

export class ElectronRuntime {
  readonly projectDir: string;
  readonly projectBase: string;
  readonly logger: Logger;
  readonly store: StreamStore;
  readonly batchStore: BatchStore;
  private readonly workItemStore: WorkItemStore;
  readonly hookEvents: HookEventStore;
  readonly resumeTracker: ResumeTracker;
  readonly lspManager: LspSessionManager;
  readonly workspaceWatchers: WorkspaceWatcherRegistry;
  readonly config: NewdeConfig;

  private readonly paneSessionFiles = new Map<string, SessionFiles>();
  private readonly terminalSessions = new Map<string, RuntimeSocket>();
  private readonly lspClients = new Map<string, RuntimeSocket>();
  private readonly hookInbox: HookInbox;
  private readonly hookInboxDir: string;
  private readonly workspaceSubs = new Set<(event: WorkspaceWatchEvent) => void>();
  private readonly hookSubs = new Set<(event: StoredEvent) => void>();
  private mcp: McpServerHandle | null = null;

  private constructor(projectDir: string, projectBase: string, logger: Logger, config: NewdeConfig) {
    this.projectDir = projectDir;
    this.projectBase = projectBase;
    this.logger = logger;
    this.config = config;
    this.store = new StreamStore(projectDir, logger.child({ subsystem: "stream-store" }));
    this.batchStore = new BatchStore(projectDir, logger.child({ subsystem: "batch-store" }));
    this.workItemStore = new WorkItemStore(projectDir, logger.child({ subsystem: "work-items" }));
    this.hookEvents = new HookEventStore(1000);
    this.resumeTracker = new ResumeTracker();
    this.lspManager = new LspSessionManager(logger.child({ subsystem: "lsp" }));
    this.workspaceWatchers = new WorkspaceWatcherRegistry(logger.child({ subsystem: "workspace-watch" }));
    this.hookInboxDir = join(projectDir, ".newde", "runtime", "hook-inbox");
    this.hookInbox = new HookInbox(this.hookInboxDir, (envelope) => this.handleHookEnvelope(envelope), logger.child({ subsystem: "hook-inbox" }));
  }

  static async create(projectDir: string): Promise<ElectronRuntime> {
    const logger = createDaemonLogger(projectDir).child({ pid: process.pid, subsystem: "electron-runtime" });
    const config = loadProjectConfig(projectDir, logger.child({ subsystem: "config" }));
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
    }

    this.workspaceWatchers.subscribe((event) => {
      for (const subscriber of this.workspaceSubs) subscriber(event);
    });
    this.hookEvents.subscribe((event) => {
      for (const subscriber of this.hookSubs) subscriber(event);
    });

    this.mcp = await startMcpServer({
      workspaceFolders: this.store.list().map((candidate) => candidate.worktree_path),
      logger: this.logger.child({ subsystem: "mcp" }),
      extraTools: this.buildMcpTools(),
    });
    this.logger.info("started mcp server", { port: this.mcp.port, lockfilePath: this.mcp.lockfilePath });

    this.hookInbox.start();
  }

  async dispose(): Promise<void> {
    cleanupSessions(this.store.list());
    this.hookInbox.dispose();
    for (const socket of this.terminalSessions.values()) socket.close();
    this.terminalSessions.clear();
    for (const socket of this.lspClients.values()) socket.close();
    this.lspClients.clear();
    this.workspaceWatchers.dispose();
    await this.lspManager.dispose();
    if (this.mcp) {
      await this.mcp.stop();
      this.mcp = null;
    }
    for (const files of this.paneSessionFiles.values()) {
      destroySessionFiles(files);
    }
    this.paneSessionFiles.clear();
    getStateDatabase(this.projectDir).close();
  }

  onWorkspaceEvent(listener: (event: WorkspaceWatchEvent) => void): () => void {
    this.workspaceSubs.add(listener);
    return () => this.workspaceSubs.delete(listener);
  }

  onHookEvent(listener: (event: StoredEvent) => void): () => void {
    this.hookSubs.add(listener);
    return () => this.hookSubs.delete(listener);
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
    const updated = this.store.update(current.id, (stream) => ({ ...stream, title }));
    this.logger.info("renamed current stream", { streamId: updated.id, title: updated.title });
    return updated;
  }

  listBranches() {
    return listBranches(this.projectDir);
  }

  getWorkspaceContext() {
    return { gitEnabled: isGitRepo(this.projectDir) };
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

  getBatchWorkState(streamId: string, batchId: string): BatchWorkState {
    this.resolveBatch(streamId, batchId);
    return this.workItemStore.getState(batchId);
  }

  createWorkItem(
    streamId: string,
    batchId: string,
    input: {
      kind: WorkItemKind;
      title: string;
      description?: string;
      parentId?: string | null;
      status?: WorkItemStatus;
      priority?: WorkItemPriority;
    },
  ): BatchWorkState {
    this.resolveBatch(streamId, batchId);
    this.workItemStore.createItem({
      batchId,
      parentId: input.parentId,
      kind: input.kind,
      title: input.title,
      description: input.description,
      status: input.status,
      priority: input.priority,
      createdBy: "user",
      actorId: "ui",
    });
    return this.workItemStore.getState(batchId);
  }

  updateWorkItem(
    streamId: string,
    batchId: string,
    itemId: string,
    changes: {
      title?: string;
      description?: string;
      parentId?: string | null;
      status?: WorkItemStatus;
      priority?: WorkItemPriority;
    },
  ): BatchWorkState {
    this.resolveBatch(streamId, batchId);
    this.workItemStore.updateItem({
      batchId,
      itemId,
      title: changes.title,
      description: changes.description,
      parentId: changes.parentId,
      status: changes.status,
      priority: changes.priority,
      actorKind: "user",
      actorId: "ui",
    });
    return this.workItemStore.getState(batchId);
  }

  addWorkItemNote(streamId: string, batchId: string, itemId: string, note: string): WorkItemEvent[] {
    this.resolveBatch(streamId, batchId);
    this.workItemStore.addNote(batchId, itemId, note, "user", "ui");
    return this.workItemStore.listEvents(batchId, itemId);
  }

  listWorkItemEvents(streamId: string, batchId: string, itemId?: string): WorkItemEvent[] {
    this.resolveBatch(streamId, batchId);
    return this.workItemStore.listEvents(batchId, itemId);
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
    const saved = writeWorkspaceFile(stream.worktree_path, path, content);
    this.workspaceWatchers.notify(stream.id, "updated", saved.path);
    return saved;
  }

  createWorkspaceFile(streamId: string, path: string, content = "") {
    const stream = this.resolveStream(streamId);
    const created = createWorkspaceFile(stream.worktree_path, path, content);
    this.workspaceWatchers.notify(stream.id, "created", created.path);
    return created;
  }

  createWorkspaceDirectory(streamId: string, path: string) {
    const stream = this.resolveStream(streamId);
    const created = createWorkspaceDirectory(stream.worktree_path, path);
    this.workspaceWatchers.notify(stream.id, "created", created.path);
    return created;
  }

  renameWorkspacePath(streamId: string, fromPath: string, toPath: string) {
    const stream = this.resolveStream(streamId);
    const renamed = renameWorkspacePath(stream.worktree_path, fromPath, toPath);
    this.workspaceWatchers.notify(stream.id, "deleted", renamed.fromPath);
    this.workspaceWatchers.notify(stream.id, "created", renamed.toPath);
    return renamed;
  }

  deleteWorkspacePath(streamId: string, path: string) {
    const stream = this.resolveStream(streamId);
    const deleted = deleteWorkspacePath(stream.worktree_path, path);
    this.workspaceWatchers.notify(stream.id, "deleted", deleted.path);
    return deleted;
  }

  listHookEvents(streamId?: string) {
    return this.hookEvents.list(streamId);
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
      const created = ensureAgentPane(
        batch.pane_target,
        stream.worktree_path,
        cols,
        rows,
        agentCommand,
        paneLogger,
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
      attachPane(socket as any, batch.pane_target, cols, rows, paneLogger.child({ subsystem: "pty-bridge", mode }));
    } else {
      this.resumeTracker.noteSessionLaunch(`${stream.id}:${batch.id}`, !!batch.resume_session_id);
      attachCommand(socket as any, agentCommand, stream.worktree_path, cols, rows, paneLogger.child({ subsystem: "pty-bridge", mode }));
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
    await this.lspManager.attachClient(socket as any, stream, languageId);
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

  private getAgentCommand(stream: Stream, batch: Batch): string {
    if (this.config.agent === "claude") {
      const key = `${stream.id}:${batch.id}`;
      let files = this.paneSessionFiles.get(key);
      if (!files) {
        files = createElectronSessionFiles({
          hookInboxDir: this.hookInboxDir,
          streamId: stream.id,
          batchId: batch.id,
        });
        this.paneSessionFiles.set(key, files);
        this.logger.info("created session files", {
          streamId: stream.id,
          batchId: batch.id,
          settingsPath: files.settingsPath,
        });
      }
      return buildAgentCommandForSession(
        this.config.agent,
        stream.worktree_path,
        batch.resume_session_id,
        files.settingsPath,
        buildBatchAgentPrompt(stream, batch),
        buildBatchMcpConfig(this.mcp),
      );
    }
    return buildAgentCommandForSession(
      this.config.agent,
      stream.worktree_path,
      batch.resume_session_id,
    );
  }

  private buildMcpTools(): ToolDef[] {
    return [
      {
        name: "newde__get_batch_context",
        description: "Return stream and batch context. Use this to confirm the active batch id before calling work-item tools.",
        inputSchema: {
          type: "object",
          properties: {
            streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
            batchId: { type: "string", description: "Optional batch id to resolve within the stream." },
          },
        },
        handler: (args: { streamId?: string; batchId?: string }) => {
          const stream = this.resolveStream(args.streamId);
          const batchState = this.batchStore.list(stream.id);
          const batch = args.batchId
            ? this.resolveBatch(stream.id, args.batchId)
            : batchState.batches.find((candidate) => candidate.id === batchState.selectedBatchId) ?? batchState.batches[0] ?? null;
          return {
            streamId: stream.id,
            streamTitle: stream.title,
            batchId: batch?.id ?? null,
            batchTitle: batch?.title ?? null,
            activeBatchId: batchState.activeBatchId,
            selectedBatchId: batchState.selectedBatchId,
          };
        },
      },
      {
        name: "newde__list_batch_work",
        description: "List all tracked work items for one batch, grouped by waiting/in progress/done. Always pass the batchId from your session context.",
        inputSchema: {
          type: "object",
          properties: {
            streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
            batchId: { type: "string", description: "Required batch id for the work you are managing." },
          },
          required: ["batchId"],
        },
        handler: (args: { streamId?: string; batchId: string }) => {
          const stream = this.resolveStream(args.streamId);
          this.resolveBatch(stream.id, args.batchId);
          return this.workItemStore.getState(args.batchId);
        },
      },
      {
        name: "newde__list_ready_work",
        description: "List actionable work items in one batch that are not blocked by unfinished dependencies. Always pass the batchId from your session context.",
        inputSchema: {
          type: "object",
          properties: {
            streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
            batchId: { type: "string", description: "Required batch id for the work you are managing." },
          },
          required: ["batchId"],
        },
        handler: (args: { streamId?: string; batchId: string }) => {
          const stream = this.resolveStream(args.streamId);
          this.resolveBatch(stream.id, args.batchId);
          return this.workItemStore.listReady(args.batchId);
        },
      },
      {
        name: "newde__create_work_item",
        description: "Create a new epic/task/subtask/bug/note within one batch. Always pass the batchId from your session context.",
        inputSchema: {
          type: "object",
          properties: {
            streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
            batchId: { type: "string", description: "Required batch id for the work you are managing." },
            parentId: { type: "string", description: "Optional parent epic/task id in the same batch." },
            kind: { type: "string", description: "One of epic, task, subtask, bug, or note." },
            title: { type: "string", description: "Short title for the work item." },
            description: { type: "string", description: "Optional longer description." },
            status: { type: "string", description: "Optional initial status." },
            priority: { type: "string", description: "Optional priority: low, medium, high, or urgent." },
          },
          required: ["batchId", "kind", "title"],
        },
        handler: (args: {
          streamId?: string;
          batchId: string;
          parentId?: string;
          kind: WorkItemKind;
          title: string;
          description?: string;
          status?: WorkItemStatus;
          priority?: WorkItemPriority;
        }) => {
          const stream = this.resolveStream(args.streamId);
          this.resolveBatch(stream.id, args.batchId);
          const item = this.workItemStore.createItem({
            batchId: args.batchId,
            parentId: args.parentId,
            kind: args.kind,
            title: args.title,
            description: args.description,
            status: args.status,
            priority: args.priority,
            createdBy: "agent",
            actorId: "mcp",
          });
          return { item, state: this.workItemStore.getState(args.batchId) };
        },
      },
      {
        name: "newde__update_work_item",
        description: "Update the title, description, status, priority, or parent of an existing work item in one batch. Always pass the batchId from your session context.",
        inputSchema: {
          type: "object",
          properties: {
            streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
            batchId: { type: "string", description: "Required batch id for the work you are managing." },
            itemId: { type: "string", description: "Required id of the work item to update." },
            parentId: { type: "string", description: "Optional new parent epic/task id." },
            title: { type: "string", description: "Optional replacement title." },
            description: { type: "string", description: "Optional replacement description." },
            status: { type: "string", description: "Optional replacement status." },
            priority: { type: "string", description: "Optional replacement priority." },
          },
          required: ["batchId", "itemId"],
        },
        handler: (args: {
          streamId?: string;
          batchId: string;
          itemId: string;
          parentId?: string | null;
          title?: string;
          description?: string;
          status?: WorkItemStatus;
          priority?: WorkItemPriority;
        }) => {
          const stream = this.resolveStream(args.streamId);
          this.resolveBatch(stream.id, args.batchId);
          const item = this.workItemStore.updateItem({
            batchId: args.batchId,
            itemId: args.itemId,
            parentId: args.parentId,
            title: args.title,
            description: args.description,
            status: args.status,
            priority: args.priority,
            actorKind: "agent",
            actorId: "mcp",
          });
          return { item, state: this.workItemStore.getState(args.batchId) };
        },
      },
      {
        name: "newde__link_work_items",
        description: "Create a relationship like blocks/relates_to/discovered_from between two work items in one batch. Always pass the batchId from your session context.",
        inputSchema: {
          type: "object",
          properties: {
            streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
            batchId: { type: "string", description: "Required batch id for the work you are managing." },
            fromItemId: { type: "string", description: "Source work item id." },
            toItemId: { type: "string", description: "Target work item id." },
            linkType: { type: "string", description: "One of blocks, relates_to, or discovered_from." },
          },
          required: ["batchId", "fromItemId", "toItemId", "linkType"],
        },
        handler: (args: {
          streamId?: string;
          batchId: string;
          fromItemId: string;
          toItemId: string;
          linkType: "blocks" | "relates_to" | "discovered_from";
        }) => {
          const stream = this.resolveStream(args.streamId);
          this.resolveBatch(stream.id, args.batchId);
          this.workItemStore.linkItems(args.batchId, args.fromItemId, args.toItemId, args.linkType);
          return { ok: true };
        },
      },
      {
        name: "newde__add_work_note",
        description: "Append a note/history entry to a work item in one batch. Always pass the batchId from your session context.",
        inputSchema: {
          type: "object",
          properties: {
            streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
            batchId: { type: "string", description: "Required batch id for the work you are managing." },
            itemId: { type: "string", description: "Optional work item id if the note belongs to one specific item." },
            note: { type: "string", description: "Required note content." },
          },
          required: ["batchId", "note"],
        },
        handler: (args: { streamId?: string; batchId: string; itemId?: string; note: string }) => {
          const stream = this.resolveStream(args.streamId);
          this.resolveBatch(stream.id, args.batchId);
          this.workItemStore.addNote(args.batchId, args.itemId ?? null, args.note, "agent", "mcp");
          return {
            ok: true,
            events: this.workItemStore.listEvents(args.batchId, args.itemId),
          };
        },
      },
    ];
  }

  private handleHookEnvelope(envelope: HookEnvelope): void {
    const stored = ingestHookPayload(this.hookEvents, envelope.event, envelope.payload, {
      streamId: envelope.streamId,
      batchId: envelope.batchId,
      pane: envelope.pane,
    });
    if (envelope.batchId && this.store.get(envelope.streamId)) {
      const update = this.resumeTracker.recordSessionHookEvent(
        `${envelope.streamId}:${envelope.batchId}`,
        envelope.event,
        stored.normalized.sessionId,
      );
      if (update?.type === "set") {
        this.batchStore.updateResume(envelope.streamId, envelope.batchId, update.sessionId);
      } else if (update?.type === "clear") {
        this.batchStore.updateResume(envelope.streamId, envelope.batchId, "");
      }
    }
  }
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

interface HookEnvelope {
  event: string;
  streamId: string;
  batchId?: string;
  pane?: PaneKind;
  payload: unknown;
}

class HookInbox {
  private watcher: FSWatcher | null = null;

  constructor(
    private readonly dir: string,
    private readonly onEnvelope: (envelope: HookEnvelope) => void,
    private readonly logger: Logger,
  ) {}

  start() {
    mkdirSync(this.dir, { recursive: true });
    this.processPending();
    this.watcher = watch(this.dir, () => {
      this.processPending();
    });
  }

  dispose() {
    this.watcher?.close();
    this.watcher = null;
  }

  private processPending() {
    for (const entry of readdirSync(this.dir)) {
      if (!entry.endsWith(".json")) continue;
      const path = join(this.dir, entry);
      try {
        const envelope = JSON.parse(readFileSync(path, "utf8")) as HookEnvelope;
        this.onEnvelope(envelope);
      } catch (error) {
        this.logger.warn("failed to process hook envelope", { file: entry, error: errorMessage(error) });
      } finally {
        try { rmSync(path, { force: true }); } catch {}
      }
    }
  }
}

function buildBatchAgentPrompt(stream: Stream, batch: Batch): string {
  return [
    `You are working in stream "${stream.title}" (id: ${stream.id}) and batch "${batch.title}" (id: ${batch.id}).`,
    "Use the newde MCP work-item tools to track planning and progress for this batch.",
    `When calling any newde work-item MCP tool, always pass batchId="${batch.id}".`,
    "Use newde__get_batch_context if you need to re-check the stream or batch ids before updating work.",
  ].join(" ");
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

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
