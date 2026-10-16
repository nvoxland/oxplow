// The chat thread view's state, as a pure reducer over the backend's
// snapshot (`chatTranscript`) and live `chat:event`s.
//
// Every transcript item has a stable `id` and a `seq` bumped from one
// session-wide counter each time it changes; the backend emits an event
// for every bump. So events for a live view arrive with consecutive
// seqs, and a jump means some were missed (a lagged socket, a
// reconnect): the state goes `stale` and the view refetches
// `since(headSeq)`, which merges back in by id. See
// .context/agent-model.md → "Chat sessions".

import type { ChatEvent, ChatSnapshot, ChatStatus, ContextUsage, TranscriptItem } from "../../api.js";

export interface ChatViewState {
  agent: string;
  /// The session generation this state holds; a new one (a Restart)
  /// starts over rather than merging, since its ids and seqs restart.
  generation: number | null;
  status: ChatStatus;
  /** Ordered by id (creation order). */
  items: TranscriptItem[];
  headSeq: number;
  usage: ContextUsage | null;
  stderrTail: string[];
  closedReason: string | null;
  /** Events were missed; refetch `since(headSeq)`. */
  stale: boolean;
}

export function initialState(): ChatViewState {
  return {
    agent: "",
    generation: null,
    status: "starting",
    items: [],
    headSeq: 0,
    usage: null,
    stderrTail: [],
    closedReason: null,
    stale: false,
  };
}

/** Insert or replace by id, keeping the newer copy, in id order. */
export function upsertItems(items: TranscriptItem[], incoming: TranscriptItem[]): TranscriptItem[] {
  const byId = new Map(items.map((i) => [i.id, i]));
  for (const item of incoming) {
    const have = byId.get(item.id);
    if (!have || item.seq > have.seq) byId.set(item.id, item);
  }
  return [...byId.values()].sort((a, b) => a.id - b.id);
}

export function fromSnapshot(s: ChatSnapshot): ChatViewState {
  return mergeSnapshot(initialState(), s);
}

/** Fold a (possibly partial, `since`) snapshot in. */
export function mergeSnapshot(state: ChatViewState, s: ChatSnapshot): ChatViewState {
  const base = sameGeneration(state, s.generation) ? state : initialState();
  return {
    ...base,
    generation: s.generation,
    agent: s.agent,
    status: s.status,
    items: upsertItems(base.items, s.items),
    headSeq: Math.max(base.headSeq, s.headSeq),
    usage: s.usage,
    stderrTail: s.stderrTail,
    stale: false,
  };
}

/// Whether `generation` is the one `state` holds (or `state` holds none yet).
function sameGeneration(state: ChatViewState, generation: number | undefined): boolean {
  return state.generation === null || generation === undefined || generation === state.generation;
}

export function applyEvent(prev: ChatViewState, e: ChatEvent): ChatViewState {
  // A new session's first event: start over (keeping the agent's name).
  const state: ChatViewState = sameGeneration(prev, e.generation)
    ? { ...prev, generation: prev.generation ?? e.generation ?? null }
    : { ...initialState(), agent: prev.agent, generation: e.generation };
  switch (e.type) {
    case "item": {
      // Past a gap the head stays put: it's the last seq known to have
      // nothing missing before it, which is what the refetch asks from.
      // A merged snapshot moves it on.
      const gap = state.stale || e.item.seq > state.headSeq + 1;
      return {
        ...state,
        items: upsertItems(state.items, [e.item]),
        headSeq: gap ? state.headSeq : Math.max(state.headSeq, e.item.seq),
        stale: gap,
      };
    }
    case "status":
      return { ...state, status: e.status };
    case "usage":
      return { ...state, usage: e.usage };
    case "closed":
      return { ...state, status: "stopped", closedReason: e.reason };
  }
}

/** A turn is in flight: no new prompt until it ends (never queued). */
export function isBusy(status: ChatStatus): boolean {
  return status === "running" || status === "awaiting_permission";
}

/** Permission cards still waiting on the person. */
export function openPermissions(state: ChatViewState): TranscriptItem[] {
  return state.items.filter((i) => i.type === "permission" && i.answer === null);
}

/** Context-window occupancy in percent, or null when unknown. */
export function contextPercent(usage: ContextUsage | null): number | null {
  if (!usage || usage.size <= 0) return null;
  return Math.round((usage.used / usage.size) * 100);
}
