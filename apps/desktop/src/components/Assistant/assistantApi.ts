/// The Assistant's backend, as the panel codes against it: the IPC contract
/// of `crates/oxplow-app/src/assistant/` (M7a). Until its generated bindings
/// exist the types are written out here and the calls go through a backend
/// that says so; wiring the real one is this file alone (`setAssistantBackend`
/// stays for tests).

// ---- the transcript (Rust `TranscriptItem` / `ItemBody`, camelCase) --------

export type ToolStatus = "pending" | "in_progress" | "completed" | "failed";

export interface AssistantToolCall {
  id: string;
  title: string;
  name: string | null;
  kind: string;
  status: ToolStatus;
  locations: string[];
  rawInput: unknown;
  rawOutput: unknown;
  text: string[];
  /** The command run's audit id once it ran and can be undone. */
  auditId?: number | null;
}

export interface PermissionOption {
  id: string;
  name: string;
  kind: "allow_once" | "allow_always" | "reject_once" | "reject_always";
}

export type PermissionAnswer = { type: "selected"; optionId: string } | { type: "cancelled" };

export type AssistantItem = { id: number; seq: number } & (
  | { type: "user"; text: string; context: string | null }
  | { type: "agent"; text: string }
  | { type: "thought"; text: string }
  | { type: "tool"; call: AssistantToolCall }
  | { type: "plan"; entries: Array<{ content: string; status: "pending" | "in_progress" | "completed" }> }
  | {
      type: "permission";
      requestId: string;
      toolCallId: string;
      title: string;
      options: PermissionOption[];
      answer: PermissionAnswer | null;
    }
  | { type: "policy_denied"; toolCallId: string; label: string; reason: string }
  | { type: "bypass"; toolCallId: string; label: string; reason: string }
  | { type: "error"; message: string }
);

// ---- the contract -----------------------------------------------------------

export type AssistantContext =
  | { kind: "page"; uri: string; label?: string }
  | { kind: "selection"; uri?: string; text: string; label?: string };

export interface ModelChoice {
  provider: string;
  model: string;
}

export interface AssistantSnapshot {
  status: "idle" | "running";
  /** Items with `seq` above the `since` asked for. */
  items: AssistantItem[];
  model: ModelChoice | null;
  models: Array<ModelChoice & { label: string }>;
  allowChanges: boolean;
  /** Why it can't answer (`no_model`: no model is bound to the role). */
  problem?: { kind: string; message: string } | null;
}

export interface AssistantBackend {
  open(args: { stream: string; thread?: string | null; context?: AssistantContext | null }): Promise<{ id: string }>;
  prompt(id: string, text: string): Promise<void>;
  cancel(id: string): Promise<void>;
  set(id: string, change: { model?: ModelChoice | null; allowChanges?: boolean }): Promise<void>;
  answer(id: string, ask: string, allow: boolean): Promise<void>;
  snapshot(id: string, since: number): Promise<AssistantSnapshot>;
  handoff(id: string, toThread: string): Promise<void>;
  close(id: string): Promise<void>;
  /** `assistantChanged { id, seq }` on `/events`. */
  subscribe(onChanged: (id: string, seq: number) => void): () => void;
}

const notWired = (): never => {
  throw new Error("The Assistant isn't available in this build yet.");
};

/** Stands in until the generated bindings are wired. */
const unwired: AssistantBackend = {
  open: async () => notWired(),
  prompt: async () => notWired(),
  cancel: async () => notWired(),
  set: async () => notWired(),
  answer: async () => notWired(),
  snapshot: async () => notWired(),
  handoff: async () => notWired(),
  close: async () => notWired(),
  subscribe: () => () => {},
};

let backend: AssistantBackend = unwired;

export function setAssistantBackend(next: AssistantBackend | null): void {
  backend = next ?? unwired;
}

export const assistantApi: AssistantBackend = {
  open: (a) => backend.open(a),
  prompt: (id, t) => backend.prompt(id, t),
  cancel: (id) => backend.cancel(id),
  set: (id, c) => backend.set(id, c),
  answer: (id, ask, allow) => backend.answer(id, ask, allow),
  snapshot: (id, since) => backend.snapshot(id, since),
  handoff: (id, to) => backend.handoff(id, to),
  close: (id) => backend.close(id),
  subscribe: (f) => backend.subscribe(f),
};
