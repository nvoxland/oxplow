import { normalize, type NormalizedEvent } from "../core/events.js";
import type { PaneKind } from "../persistence/stream-store.js";

export interface StoredEvent {
  id: number;
  streamId: string;
  batchId?: string;
  pane?: PaneKind;
  normalized: NormalizedEvent;
}

export class HookEventStore {
  private buf = new Map<string, StoredEvent[]>();
  private nextId = 1;
  private subs = new Set<(e: StoredEvent) => void>();
  constructor(private capacity: number) {}

  push(streamId: string, event: string, payload: any, pane?: PaneKind, batchId?: string): StoredEvent {
    const normalized = normalize(event, payload, Date.now());
    const item: StoredEvent = { id: this.nextId++, streamId, batchId, pane, normalized };
    const bucket = this.buf.get(streamId) ?? [];
    bucket.push(item);
    if (bucket.length > this.capacity) {
      bucket.splice(0, bucket.length - this.capacity);
    }
    this.buf.set(streamId, bucket);
    for (const fn of this.subs) {
      try { fn(item); } catch {}
    }
    return item;
  }

  list(streamId?: string): StoredEvent[] {
    if (streamId) return (this.buf.get(streamId) ?? []).slice();
    return [...this.buf.values()].flatMap((items) => items.slice()).sort((a, b) => a.id - b.id);
  }

  subscribe(fn: (e: StoredEvent) => void, streamId?: string): () => void {
    const wrapped = (evt: StoredEvent) => {
      if (streamId && evt.streamId !== streamId) return;
      fn(evt);
    };
    this.subs.add(wrapped);
    return () => this.subs.delete(wrapped);
  }
}

export function ingestHookPayload(
  store: HookEventStore,
  event: string,
  payload: any,
  context: { streamId?: string; pane?: PaneKind; batchId?: string } = {},
): StoredEvent {
  return store.push(context.streamId ?? "default", event, payload, context.pane, context.batchId);
}
