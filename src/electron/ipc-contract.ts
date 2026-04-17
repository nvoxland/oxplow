import type {
  BranchRef,
  Batch,
  BatchState,
  GitFileStatus,
  BatchWorkState,
  Stream,
  WorkItemEvent,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
  WorkspaceContext,
  WorkspaceEntry,
  WorkspaceFile,
  WorkspaceIndexedFile,
  WorkspacePathChange,
  WorkspaceRenameResult,
  WorkspaceStatusSummary,
  WorkspaceWatchEvent,
  StoredEvent,
} from "../ui/api.js";
import type { AgentStatus, NewdeEvent } from "../core/event-bus.js";
import type { CommandId, MenuGroupSnapshot } from "../ui/commands.js";

export type {
  BranchRef,
  Batch,
  BatchState,
  BatchWorkState,
  CommandId,
  GitFileStatus,
  AgentStatus,
  MenuGroupSnapshot,
  NewdeEvent,
  Stream,
  WorkItemEvent,
  WorkItemKind,
  WorkItemPriority,
  WorkItemStatus,
  WorkspaceContext,
  WorkspaceEntry,
  WorkspaceFile,
  WorkspaceIndexedFile,
  WorkspacePathChange,
  WorkspaceRenameResult,
  WorkspaceStatusSummary,
  WorkspaceWatchEvent,
  StoredEvent,
};

export interface UiLogPayload {
  clientId: string;
  level: "debug" | "info" | "warn" | "error";
  message: string;
  context?: Record<string, unknown>;
  timestamp?: string;
}

export interface TerminalEvent {
  sessionId: string;
  message: string;
}

export interface LspEvent {
  clientId: string;
  message: string;
}

export interface DesktopApi {
  getCurrentStream(): Promise<Stream>;
  listStreams(): Promise<Stream[]>;
  switchStream(id: string): Promise<Stream>;
  renameCurrentStream(title: string): Promise<Stream>;
  listBranches(): Promise<BranchRef[]>;
  getWorkspaceContext(): Promise<WorkspaceContext>;
  createStream(input:
    | { title: string; summary?: string; source: "existing"; ref: string }
    | { title: string; summary?: string; source: "new"; branch: string; startPointRef: string },
  ): Promise<Stream>;
  getBatchState(streamId: string): Promise<BatchState>;
  createBatch(streamId: string, title: string): Promise<BatchState>;
  reorderBatch(streamId: string, batchId: string, targetIndex: number): Promise<BatchState>;
  selectBatch(streamId: string, batchId: string): Promise<BatchState>;
  promoteBatch(streamId: string, batchId: string): Promise<BatchState>;
  completeBatch(streamId: string, batchId: string): Promise<BatchState>;
  getBatchWorkState(streamId: string, batchId: string): Promise<BatchWorkState>;
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
  ): Promise<BatchWorkState>;
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
  ): Promise<BatchWorkState>;
  deleteWorkItem(streamId: string, batchId: string, itemId: string): Promise<BatchWorkState>;
  reorderWorkItems(streamId: string, batchId: string, orderedItemIds: string[]): Promise<BatchWorkState>;
  addWorkItemNote(streamId: string, batchId: string, itemId: string, note: string): Promise<WorkItemEvent[]>;
  listWorkItemEvents(streamId: string, batchId: string, itemId?: string): Promise<WorkItemEvent[]>;
  listWorkspaceEntries(streamId: string, path?: string): Promise<WorkspaceEntry[]>;
  listWorkspaceFiles(streamId: string): Promise<{ files: WorkspaceIndexedFile[]; summary: WorkspaceStatusSummary }>;
  readWorkspaceFile(streamId: string, path: string): Promise<WorkspaceFile>;
  writeWorkspaceFile(streamId: string, path: string, content: string): Promise<WorkspaceFile>;
  createWorkspaceFile(streamId: string, path: string, content?: string): Promise<WorkspaceFile>;
  createWorkspaceDirectory(streamId: string, path: string): Promise<WorkspacePathChange>;
  renameWorkspacePath(streamId: string, fromPath: string, toPath: string): Promise<WorkspaceRenameResult>;
  deleteWorkspacePath(streamId: string, path: string): Promise<WorkspacePathChange>;
  listHookEvents(streamId?: string): Promise<StoredEvent[]>;
  listAgentStatuses(streamId?: string): Promise<Array<{ streamId: string; batchId: string; status: AgentStatus }>>;
  ping(): Promise<boolean>;
  logUi(payload: UiLogPayload): Promise<void>;
  setNativeMenu(groups: MenuGroupSnapshot[]): Promise<void>;
  openTerminalSession(paneTarget: string, cols: number, rows: number, mode?: "direct" | "tmux"): Promise<string>;
  sendTerminalMessage(sessionId: string, message: string): Promise<void>;
  closeTerminalSession(sessionId: string): Promise<void>;
  openLspClient(streamId: string, languageId: string): Promise<string>;
  sendLspMessage(clientId: string, message: string): Promise<void>;
  closeLspClient(clientId: string): Promise<void>;
  onNewdeEvent(listener: (event: NewdeEvent) => void): () => void;
  onTerminalEvent(listener: (event: TerminalEvent) => void): () => void;
  onLspEvent(listener: (event: LspEvent) => void): () => void;
  onMenuCommand(listener: (commandId: CommandId) => void): () => void;
}
