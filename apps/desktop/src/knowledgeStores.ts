import { onRemoteReconnect, subscribeOxplowEvents } from "./api.js";
import { readKnowledgeStores, type KnowledgeStore } from "./knowledge.js";
import { setKnowledgeStoreIds } from "./knowledgeStoreIds.js";
import { coalesce, NO_READS, readsChanged } from "./viz/rerun.js";

/**
 * The knowledge stores, shared by the knowledge surfaces: read
 * once, re-read when the model's stores change (a store's extension turned
 * on or off). Empty until the first read lands. Components use
 * `useKnowledgeStores`; markdown reads the ids (`knowledgeStoreIds.ts`).
 */

let stores: KnowledgeStore[] = [];
let reads = NO_READS;
let started = false;
const listeners = new Set<() => void>();

async function refresh(): Promise<void> {
  try {
    const out = await readKnowledgeStores();
    publish(out.stores);
    reads = out.reads;
  } catch {
    // Keep what we had; the next change re-reads.
  }
}

function publish(next: KnowledgeStore[]): void {
  stores = next;
  setKnowledgeStoreIds(next.map((s) => ({ store: s.store, idPattern: s.idPattern })));
  for (const fn of listeners) fn();
}

/** Start the shared read (the app does at boot, so markdown knows the
 *  stores before any knowledge surface mounts). */
export function startKnowledgeStores(): void {
  start();
}

function start(): void {
  if (started) return;
  started = true;
  void refresh();
  const rerun = coalesce(() => void refresh());
  subscribeOxplowEvents((event) => {
    if (readsChanged(event as Record<string, unknown>, reads)) rerun.schedule();
  });
  onRemoteReconnect(rerun.schedule);
}

/** The enabled stores now (it starts the shared read). */
export function knowledgeStores(): KnowledgeStore[] {
  start();
  return stores;
}

/** Set the enabled stores (a test's, or a read that already holds them). */
export function setKnowledgeStores(next: KnowledgeStore[]): void {
  publish(next);
}

/** Hear every change of the enabled stores; the returned function stops. */
export function onKnowledgeStores(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
