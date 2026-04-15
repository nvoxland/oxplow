export interface Stream {
  id: string;
  title: string;
  summary: string;
  branch: string;
  created_at: string;
  updated_at: string;
  panes: { working: string; talking: string };
}

export async function getCurrentStream(): Promise<Stream> {
  const r = await fetch("/api/streams/current");
  if (!r.ok) throw new Error(`failed: ${r.status}`);
  return r.json();
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
  normalized: NormalizedEvent;
}
