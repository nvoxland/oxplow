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
  return fetchJson("/api/streams/current");
}

export async function listStreams(): Promise<Stream[]> {
  return fetchJson("/api/streams");
}

export async function switchStream(id: string): Promise<Stream> {
  return fetchJson("/api/streams/current", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ id }),
  });
}

export async function renameCurrentStream(title: string): Promise<Stream> {
  return fetchJson("/api/streams/current", {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ title }),
  });
}

export async function listBranches(): Promise<BranchRef[]> {
  return fetchJson("/api/branches");
}

export async function getWorkspaceContext(): Promise<WorkspaceContext> {
  return fetchJson("/api/workspace/context");
}

export async function createStream(input:
  | { title: string; summary?: string; source: "existing"; ref: string }
  | { title: string; summary?: string; source: "new"; branch: string; startPointRef: string },
): Promise<Stream> {
  return fetchJson("/api/streams", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
  });
}

export async function listWorkspaceEntries(streamId: string, path = ""): Promise<WorkspaceEntry[]> {
  const params = new URLSearchParams({ stream: streamId, path });
  const result = await fetchJson<{ entries: WorkspaceEntry[] }>(`/api/workspace/entries?${params.toString()}`);
  return result.entries;
}

export async function listWorkspaceFiles(streamId: string): Promise<{
  files: WorkspaceIndexedFile[];
  summary: WorkspaceStatusSummary;
}> {
  const params = new URLSearchParams({ stream: streamId });
  return fetchJson(`/api/workspace/files?${params.toString()}`);
}

export async function readWorkspaceFile(streamId: string, path: string): Promise<WorkspaceFile> {
  const params = new URLSearchParams({ stream: streamId, path });
  return fetchJson(`/api/workspace/file?${params.toString()}`);
}

export async function writeWorkspaceFile(streamId: string, path: string, content: string): Promise<WorkspaceFile> {
  const params = new URLSearchParams({ stream: streamId });
  return fetchJson(`/api/workspace/file?${params.toString()}`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ path, content }),
  });
}

export function subscribeWorkspaceEvents(
  streamId: string,
  onEvent: (event: WorkspaceWatchEvent) => void,
): () => void {
  const params = new URLSearchParams({ stream: streamId });
  const es = new EventSource(`/api/workspace/watch?${params.toString()}`);
  es.onmessage = (message) => {
    try {
      onEvent(JSON.parse(message.data) as WorkspaceWatchEvent);
    } catch {}
  };
  es.onerror = () => {
    // EventSource auto-reconnects.
  };
  return () => es.close();
}

export async function probeDaemon(): Promise<boolean> {
  try {
    const r = await fetch("/api/streams", { cache: "no-store" });
    return r.ok;
  } catch {
    return false;
  }
}

// Mirror of src/events.ts NormalizedEvent — UI side.
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

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  const r = await fetch(url, init);
  if (!r.ok) {
    let message = `failed: ${r.status}`;
    try {
      const body = await r.json();
      if (body && typeof body.error === "string") message = body.error;
    } catch {}
    throw new Error(message);
  }
  return r.json();
}
