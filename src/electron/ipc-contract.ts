import type {
  BranchRef,
  GitFileStatus,
  Stream,
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
import type { CommandId, MenuGroupSnapshot } from "../ui/commands.js";

export type {
  BranchRef,
  CommandId,
  GitFileStatus,
  MenuGroupSnapshot,
  Stream,
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
  listWorkspaceEntries(streamId: string, path?: string): Promise<WorkspaceEntry[]>;
  listWorkspaceFiles(streamId: string): Promise<{ files: WorkspaceIndexedFile[]; summary: WorkspaceStatusSummary }>;
  readWorkspaceFile(streamId: string, path: string): Promise<WorkspaceFile>;
  writeWorkspaceFile(streamId: string, path: string, content: string): Promise<WorkspaceFile>;
  createWorkspaceFile(streamId: string, path: string, content?: string): Promise<WorkspaceFile>;
  createWorkspaceDirectory(streamId: string, path: string): Promise<WorkspacePathChange>;
  renameWorkspacePath(streamId: string, fromPath: string, toPath: string): Promise<WorkspaceRenameResult>;
  deleteWorkspacePath(streamId: string, path: string): Promise<WorkspacePathChange>;
  listHookEvents(streamId?: string): Promise<StoredEvent[]>;
  ping(): Promise<boolean>;
  logUi(payload: UiLogPayload): Promise<void>;
  setNativeMenu(groups: MenuGroupSnapshot[]): Promise<void>;
  openTerminalSession(paneTarget: string, cols: number, rows: number): Promise<string>;
  sendTerminalMessage(sessionId: string, message: string): Promise<void>;
  closeTerminalSession(sessionId: string): Promise<void>;
  openLspClient(streamId: string, languageId: string): Promise<string>;
  sendLspMessage(clientId: string, message: string): Promise<void>;
  closeLspClient(clientId: string): Promise<void>;
  onWorkspaceEvent(listener: (event: WorkspaceWatchEvent) => void): () => void;
  onHookEvent(listener: (event: StoredEvent) => void): () => void;
  onTerminalEvent(listener: (event: TerminalEvent) => void): () => void;
  onLspEvent(listener: (event: LspEvent) => void): () => void;
  onMenuCommand(listener: (commandId: CommandId) => void): () => void;
}
