/// The window's one Assistant session: which it is, what it was opened
/// about, and its state (kept here so the dock icon's badge counts a turn
/// running while the panel is closed). An external store; the panel and the
/// badge read it with `useAssistant`. Sessions live in the backend's memory
/// and are gone on restart, so nothing here is persisted.

import { useSyncExternalStore } from "react";

import { recordOpError } from "../../opErrors.js";
import { dockStore } from "../Dock/dockStore.js";
import { assistantApi, type AssistantContext, type ModelChoice } from "./assistantApi.js";
import { hasTurns, initialState, mergeSnapshot, type AssistantState } from "./assistantModel.js";

export interface AssistantView {
  id: string | null;
  context: AssistantContext | null;
  state: AssistantState;
  /** Text to put in the prompt box (a chip, a typed question): never sent. */
  draft: { text: string; n: number } | null;
  /** The session is being opened. */
  opening: boolean;
  /** Opening or a call failed, said in the panel. */
  error: string | null;
}

const idle = (): AssistantView => ({ id: null, context: null, state: initialState(), opening: false, error: null, draft: null });

let view: AssistantView = idle();
let scope: { stream: string | null; thread: string | null } = { stream: null, thread: null };
const listeners = new Set<() => void>();
let unsubscribe: (() => void) | null = null;

function set(next: Partial<AssistantView>): void {
  view = { ...view, ...next };
  for (const l of [...listeners]) l();
}

export function subscribeAssistant(l: () => void): () => void {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
}

export function getAssistant(): AssistantView {
  return view;
}

export function useAssistant(): AssistantView {
  return useSyncExternalStore(subscribeAssistant, getAssistant);
}

/** The stream and thread on screen (`App` sets them as they change). */
export function setAssistantScope(next: { stream: string | null; thread: string | null }): void {
  scope = next;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

// The backend announces a change on every text delta: fetches are
// coalesced, at most one in flight and one more after it if anything
// arrived meanwhile.
let fetching: Promise<void> | null = null;
let again = false;

/** Fetch what changed in the session since the last seq. */
export function refreshAssistant(): Promise<void> {
  if (fetching) {
    again = true;
    return fetching;
  }
  fetching = (async () => {
    try {
      do {
        again = false;
        const id = view.id;
        if (!id) return;
        try {
          const snap = await assistantApi.snapshot(id, view.state.headSeq);
          if (view.id === id) set({ state: mergeSnapshot(view.state, snap), error: null });
        } catch (e) {
          if (view.id === id) set({ error: message(e) });
        }
      } while (again);
    } finally {
      fetching = null;
    }
  })();
  return fetching;
}

/** Open a session about `context` (none: a bare one). */
async function openSession(context: AssistantContext | null): Promise<void> {
  if (!scope.stream) return;
  const old = view.id;
  unsubscribe?.();
  unsubscribe = null;
  set({ ...idle(), context, draft: view.draft, opening: true });
  if (old) void assistantApi.close(old).catch(() => {});
  try {
    const { id } = await assistantApi.open({ stream: scope.stream, thread: scope.thread, context });
    set({ id, opening: false });
    unsubscribe = assistantApi.subscribe(id, () => void refreshAssistant());
    await refreshAssistant();
  } catch (e) {
    set({ opening: false, error: message(e) });
  }
}

/** Ask the Assistant about `context`: open the panel, on a new session when
 *  the current one has turns (or was opened about something else). */
export async function askAssistant(context: AssistantContext | null, draft?: string): Promise<void> {
  dockStore.open("assistant");
  if (draft) set({ draft: { text: draft, n: (view.draft?.n ?? 0) + 1 } });
  if (view.id && !hasTurns(view.state) && sameContext(view.context, context)) return;
  if (!view.id || context !== null || hasTurns(view.state)) await openSession(context);
}

/** The panel opened with nothing asked about: a session if none is. */
export async function ensureAssistant(): Promise<void> {
  if (!view.id && !view.opening) await openSession(null);
}

/** New: drop the conversation and start over, about the same thing. */
export async function newAssistant(): Promise<void> {
  await openSession(view.context);
}

export function sameContext(a: AssistantContext | null, b: AssistantContext | null): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

// ---- calls on the open session ------------------------------------------------

async function call(f: (id: string) => Promise<unknown>): Promise<void> {
  const id = view.id;
  if (!id) return;
  try {
    await f(id);
    await refreshAssistant();
  } catch (e) {
    set({ error: message(e) });
    recordOpError({ label: "Assistant", message: message(e) });
  }
}

export const sendPrompt = (text: string) => call((id) => assistantApi.prompt(id, text));
export const cancelTurn = () => call((id) => assistantApi.cancel(id));
export const answerCard = (ask: string, allow: boolean) => call((id) => assistantApi.answer(id, ask, allow));
export const chooseModel = (model: ModelChoice | null) => call((id) => assistantApi.set(id, { model }));
export const setAllowChanges = (allowChanges: boolean) => call((id) => assistantApi.set(id, { allowChanges }));
export const continueIn = (thread: string) => call((id) => assistantApi.handoff(id, thread));

export function dismissError(): void {
  set({ error: null });
}

/** Forget the session (tests). */
export function resetAssistant(): void {
  unsubscribe?.();
  unsubscribe = null;
  view = idle();
  for (const l of [...listeners]) l();
}
