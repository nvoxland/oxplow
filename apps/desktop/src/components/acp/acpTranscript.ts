// The ACP thread view's state, as a pure reducer over the backend's
// snapshot (`acpTranscript`) and live `acp:event`s.
//
// Every transcript item has a stable `id` and a `seq` bumped from one
// session-wide counter each time it changes; the backend emits an event
// for every bump. So events for a live view arrive with consecutive
// seqs, and a jump means some were missed (a lagged socket, a
// reconnect): the state goes `stale` and the view refetches
// `since(headSeq)`, which merges back in by id. See
// .context/agent-model.md → "ACP agents".

import type { AcpEvent, AcpSnapshot, AcpStatus, ContextUsage, TranscriptItem } from "../../api.js";

export interface AcpViewState {
  agent: string;
  status: AcpStatus;
  /** Ordered by id (creation order). */
  items: TranscriptItem[];
  headSeq: number;
  directive: string | null;
  usage: ContextUsage | null;
  stderrTail: string[];
  closedReason: string | null;
  /** Events were missed; refetch `since(headSeq)`. */
  stale: boolean;
}

export function initialState(): AcpViewState {
  return {
    agent: "",
    status: "starting",
    items: [],
    headSeq: 0,
    directive: null,
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

export function fromSnapshot(s: AcpSnapshot): AcpViewState {
  return mergeSnapshot(initialState(), s);
}

/** Fold a (possibly partial, `since`) snapshot in. */
export function mergeSnapshot(state: AcpViewState, s: AcpSnapshot): AcpViewState {
  return {
    ...state,
    agent: s.agent,
    status: s.status,
    items: upsertItems(state.items, s.items),
    headSeq: Math.max(state.headSeq, s.headSeq),
    directive: s.directive,
    usage: s.usage,
    stderrTail: s.stderrTail,
    stale: false,
  };
}

export function applyEvent(state: AcpViewState, e: AcpEvent): AcpViewState {
  switch (e.type) {
    case "item": {
      const gap = e.item.seq > state.headSeq + 1;
      return {
        ...state,
        items: upsertItems(state.items, [e.item]),
        headSeq: Math.max(state.headSeq, e.item.seq),
        stale: state.stale || gap,
      };
    }
    case "status":
      return { ...state, status: e.status };
    case "directive":
      return { ...state, directive: e.text };
    case "usage":
      return { ...state, usage: e.usage };
    case "closed":
      return { ...state, status: "stopped", closedReason: e.reason };
  }
}

/** A turn is in flight: no new prompt until it ends (never queued). */
export function isBusy(status: AcpStatus): boolean {
  return status === "running" || status === "awaiting_permission";
}

/** Permission cards still waiting on the person. */
export function openPermissions(state: AcpViewState): TranscriptItem[] {
  return state.items.filter((i) => i.type === "permission" && i.answer === null);
}

/** Context-window occupancy in percent, or null when unknown. */
export function contextPercent(usage: ContextUsage | null): number | null {
  if (!usage || usage.size <= 0) return null;
  return Math.round((usage.used / usage.size) * 100);
}
