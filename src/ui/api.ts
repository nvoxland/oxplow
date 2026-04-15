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
