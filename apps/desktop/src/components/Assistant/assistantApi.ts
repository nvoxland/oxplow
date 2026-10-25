/// The Assistant's backend as the panel calls it: `api.ts`'s wrappers over
/// the generated bindings (`oxplow_app::assistant`), behind an interface so
/// a test can put a scripted one in its place (`setAssistantBackend`). The
/// transcript types are the generated ones.

import {
  assistantAnswer,
  assistantCancel,
  assistantClose,
  assistantHandoff,
  assistantOpen,
  assistantPrompt,
  assistantSet,
  assistantSnapshot,
  subscribeAssistant,
} from "../../api.js";
import type {
  AssistantContext,
  AssistantModel,
  AssistantSnapshot,
  ToolCall,
  TranscriptItem,
} from "../../tauri-bridge/generated/bindings.js";

export type { AssistantContext, AssistantModel, AssistantSnapshot, ToolCall, TranscriptItem };
export type AssistantItem = TranscriptItem;
export type AssistantToolCall = ToolCall;
export type ModelChoice = AssistantModel;

export interface AssistantBackend {
  open(args: { stream: string; thread?: string | null; context?: AssistantContext | null }): Promise<{ id: string }>;
  prompt(id: string, text: string): Promise<void>;
  cancel(id: string): Promise<void>;
  /** `model: null` goes back to the `assistant` role's model. */
  set(id: string, change: { model?: ModelChoice | null; allowChanges?: boolean }): Promise<void>;
  answer(id: string, ask: string, allow: boolean): Promise<void>;
  snapshot(id: string, since: number): Promise<AssistantSnapshot>;
  handoff(id: string, toThread: string): Promise<void>;
  close(id: string): Promise<void>;
  /** `assistantChanged { id, seq }` for session `id`. */
  subscribe(id: string, onChanged: (seq: number) => void): () => void;
}

const real: AssistantBackend = {
  open: async ({ stream, thread, context }) => ({ id: await assistantOpen(stream, thread ?? null, context ?? null) }),
  prompt: assistantPrompt,
  cancel: assistantCancel,
  set: assistantSet,
  answer: assistantAnswer,
  snapshot: assistantSnapshot,
  handoff: assistantHandoff,
  close: assistantClose,
  subscribe: subscribeAssistant,
};

let backend: AssistantBackend = real;

export function setAssistantBackend(next: AssistantBackend | null): void {
  backend = next ?? real;
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
  subscribe: (id, f) => backend.subscribe(id, f),
};
