/// The Assistant panel's state as a pure model over the backend's snapshots
/// (`assistant_snapshot { id, since }`). Every item has a stable `id` and a
/// `seq` bumped each time it changes, so a `since` snapshot upserts by id.

import type { AssistantItem, AssistantSnapshot, ModelChoice } from "./assistantApi.js";

export interface AssistantState {
  status: "idle" | "running";
  /** Ordered by id (creation order). */
  items: AssistantItem[];
  headSeq: number;
  model: ModelChoice | null;
  models: AssistantSnapshot["models"];
  allowChanges: boolean;
  problem: AssistantSnapshot["problem"];
}

export function initialState(): AssistantState {
  return { status: "idle", items: [], headSeq: 0, model: null, models: [], allowChanges: false, problem: null };
}

/** Insert or replace by id, keeping the newer copy, in id order. */
export function upsertItems(items: AssistantItem[], incoming: AssistantItem[]): AssistantItem[] {
  const byId = new Map(items.map((i) => [i.id, i]));
  for (const item of incoming) {
    const have = byId.get(item.id);
    if (!have || item.seq > have.seq) byId.set(item.id, item);
  }
  return [...byId.values()].sort((a, b) => a.id - b.id);
}

export function mergeSnapshot(state: AssistantState, s: AssistantSnapshot): AssistantState {
  const items = upsertItems(state.items, s.items);
  return {
    status: s.status,
    items,
    headSeq: Math.max(state.headSeq, s.seq),
    model: s.model,
    models: s.models,
    allowChanges: s.allowChanges,
    problem: s.problem ?? null,
  };
}

/** Permission cards still waiting on the person. */
export function openPermissions(state: AssistantState): Extract<AssistantItem, { type: "permission" }>[] {
  return state.items.filter((i): i is Extract<AssistantItem, { type: "permission" }> => i.type === "permission" && i.answer === null);
}

/** A turn is in flight: no new prompt until it ends (never queued). */
export function isBusy(state: AssistantState): boolean {
  return state.status === "running" || openPermissions(state).length > 0;
}

/** The session has turns: a new context starts a new session. */
export function hasTurns(state: AssistantState): boolean {
  return state.items.some((i) => i.type === "user");
}

export type PanelMode = "no_model" | "empty" | "running" | "waiting" | "idle";

/** What the panel shows: the model's absence first, then a card waiting,
 *  a turn running, no turns yet (the suggestions), else the conversation. */
export function panelMode(state: AssistantState): PanelMode {
  if (state.problem?.kind === "no_model") return "no_model";
  if (openPermissions(state).length > 0) return "waiting";
  if (state.status === "running") return "running";
  return hasTurns(state) ? "idle" : "empty";
}

/** The dock icon's badge: a turn running or a card waiting. */
export function badgeCount(state: AssistantState): number {
  const waiting = openPermissions(state).length;
  return waiting > 0 ? waiting : state.status === "running" ? 1 : 0;
}

/** Tool calls that ran and can be undone, newest first ("Undo this turn"
 *  undoes them in reverse order). */
export function undoableCalls(state: AssistantState): Array<{ itemId: number; auditId: number }> {
  const out: Array<{ itemId: number; auditId: number }> = [];
  for (const i of state.items) {
    if (i.type === "tool" && i.call.auditId != null && i.call.status === "completed") out.push({ itemId: i.id, auditId: i.call.auditId });
  }
  return out.reverse();
}

/** Tool calls of the latest turn only (after the last user item). */
export function lastTurnUndoable(state: AssistantState): Array<{ itemId: number; auditId: number }> {
  const lastUser = [...state.items].reverse().find((i) => i.type === "user")?.id ?? 0;
  const inTurn = new Set(state.items.filter((i) => i.id > lastUser).map((i) => i.id));
  return undoableCalls(state).filter((c) => inTurn.has(c.itemId));
}
