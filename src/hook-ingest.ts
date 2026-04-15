import { normalize, type NormalizedEvent } from "./events.js";

export interface StoredEvent {
  id: number;
  normalized: NormalizedEvent;
}

export class HookEventStore {
  private buf: StoredEvent[] = [];
  private nextId = 1;
  private subs = new Set<(e: StoredEvent) => void>();
  constructor(private capacity: number) {}

  push(event: string, payload: any): StoredEvent {
    const normalized = normalize(event, payload, Date.now());
    const item: StoredEvent = { id: this.nextId++, normalized };
    this.buf.push(item);
    if (this.buf.length > this.capacity) {
      this.buf.splice(0, this.buf.length - this.capacity);
    }
    for (const fn of this.subs) {
      try { fn(item); } catch {}
    }
    return item;
  }

  list(): StoredEvent[] {
    return this.buf.slice();
  }

  subscribe(fn: (e: StoredEvent) => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }
}

export function ingestHookPayload(store: HookEventStore, event: string, payload: any): StoredEvent {
  return store.push(event, payload);
}
