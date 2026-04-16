import type { DesktopApi } from "../electron/ipc-contract.js";

export interface Stream {
  id: string;
  title: string;
  summary: string;
  branch: string;
  branch_ref: string;
  branch_source: "local" | "remote" | "new";
  worktree_path: string;
  created_at: string;
  updated_at: string;
  panes: { working: string; talking: string };
  resume: { working_session_id: string; talking_session_id: string };
}

export interface BranchRef {
  kind: "local" | "remote";
  name: string;
  ref: string;
  remote?: string;
}

export type GitFileStatus = "modified" | "added" | "deleted" | "renamed" | "untracked";

export interface WorkspaceEntry {
  name: string;
  path: string;
  kind: "file" | "directory";
  gitStatus: GitFileStatus | null;
  hasChanges: boolean;
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

export interface WorkspaceIndexedFile {
  path: string;
  gitStatus: GitFileStatus | null;
}

export interface WorkspaceStatusSummary {
  modified: number;
  added: number;
  deleted: number;
  renamed: number;
  untracked: number;
  total: number;
}

export interface WorkspaceContext {
  gitEnabled: boolean;
}

export interface WorkspaceWatchEvent {
  id: number;
  streamId: string;
  path: string;
  kind: "created" | "updated" | "deleted";
  t: number;
}

export async function getCurrentStream(): Promise<Stream> {
  return desktopApi().getCurrentStream();
}

export async function listStreams(): Promise<Stream[]> {
  return desktopApi().listStreams();
}

export async function switchStream(id: string): Promise<Stream> {
  return desktopApi().switchStream(id);
}

export async function renameCurrentStream(title: string): Promise<Stream> {
  return desktopApi().renameCurrentStream(title);
}

export async function listBranches(): Promise<BranchRef[]> {
  return desktopApi().listBranches();
}

export async function getWorkspaceContext(): Promise<WorkspaceContext> {
  return desktopApi().getWorkspaceContext();
}

export async function createStream(input:
  | { title: string; summary?: string; source: "existing"; ref: string }
  | { title: string; summary?: string; source: "new"; branch: string; startPointRef: string },
): Promise<Stream> {
  return desktopApi().createStream(input);
}

export async function listWorkspaceEntries(streamId: string, path = ""): Promise<WorkspaceEntry[]> {
  return desktopApi().listWorkspaceEntries(streamId, path);
}

export async function listWorkspaceFiles(streamId: string): Promise<{
  files: WorkspaceIndexedFile[];
  summary: WorkspaceStatusSummary;
}> {
  return desktopApi().listWorkspaceFiles(streamId);
}

export async function readWorkspaceFile(streamId: string, path: string): Promise<WorkspaceFile> {
  return desktopApi().readWorkspaceFile(streamId, path);
}

export async function writeWorkspaceFile(streamId: string, path: string, content: string): Promise<WorkspaceFile> {
  return desktopApi().writeWorkspaceFile(streamId, path, content);
}

export async function createWorkspaceFile(streamId: string, path: string, content = ""): Promise<WorkspaceFile> {
  return desktopApi().createWorkspaceFile(streamId, path, content);
}

export async function createWorkspaceDirectory(streamId: string, path: string): Promise<WorkspacePathChange> {
  return desktopApi().createWorkspaceDirectory(streamId, path);
}

export async function renameWorkspacePath(
  streamId: string,
  fromPath: string,
  toPath: string,
): Promise<WorkspaceRenameResult> {
  return desktopApi().renameWorkspacePath(streamId, fromPath, toPath);
}

export async function deleteWorkspacePath(streamId: string, path: string): Promise<WorkspacePathChange> {
  return desktopApi().deleteWorkspacePath(streamId, path);
}

export function subscribeWorkspaceEvents(
  streamId: string,
  onEvent: (event: WorkspaceWatchEvent) => void,
): () => void {
  return desktopApi().onWorkspaceEvent((event) => {
    if (event.streamId === streamId) {
      onEvent(event);
    }
  });
}

export async function probeDaemon(): Promise<boolean> {
  try {
    return await desktopApi().ping();
  } catch {
    return false;
  }
}

export type NormalizedEvent =
  | { kind: "session-start"; t: number; sessionId?: string; cwd?: string }
  | { kind: "session-end"; t: number; sessionId?: string; reason?: string }
  | { kind: "user-prompt"; t: number; sessionId?: string; prompt: string }
  | {
      kind: "tool-use-start";
      t: number;
      sessionId?: string;
      toolName: string;
      target?: string;
      input?: unknown;
    }
  | {
      kind: "tool-use-end";
      t: number;
      sessionId?: string;
      toolName: string;
      status: "ok" | "error";
    }
  | { kind: "stop"; t: number; sessionId?: string }
  | { kind: "notification"; t: number; sessionId?: string; message: string }
  | { kind: "meta"; t: number; sessionId?: string; hookEventName: string; raw: unknown };

export interface StoredEvent {
  id: number;
  streamId: string;
  pane?: "working" | "talking";
  normalized: NormalizedEvent;
}

export async function listHookEvents(streamId?: string): Promise<StoredEvent[]> {
  return desktopApi().listHookEvents(streamId);
}

export function subscribeHookEvents(
  streamId: string | "all",
  onEvent: (event: StoredEvent) => void,
): () => void {
  return desktopApi().onHookEvent((event) => {
    if (streamId === "all" || event.streamId === streamId) {
      onEvent(event);
    }
  });
}

function desktopApi(): DesktopApi {
  if (!window.newdeApi) {
    throw new Error("newde Electron API is unavailable");
  }
  return window.newdeApi;
}
