import { useCallback, useSyncExternalStore } from "react";

import { listExtensions, onRemoteReconnect, subscribeOxplowEvents, type Extension } from "./api.js";
import { recordOpError } from "./components/opErrorsStore.js";
import { extensionsChanged } from "./lens/lensRerun.js";

/**
 * The loaded extensions, per stream, shared by every component that
 * reads them: one `listExtensions` per stream and one event subscription,
 * however many menus, slots and decorators are mounted. An entry lives
 * while something reads it and is dropped with its last reader, so a
 * later mount loads afresh. Every entry reloads when the extensions'
 * definitions change (`extensionsChanged`).
 */
interface Entry {
  value: Extension[] | null;
  listeners: Set<() => void>;
  /** The latest load; an older answer is dropped. */
  seq: number;
}

const entries = new Map<string, Entry>();
let offEvents: (() => void) | null = null;

function load(key: string, entry: Entry) {
  const seq = ++entry.seq;
  const current = () => entries.get(key) === entry && entry.seq === seq;
  const settle = (value: Extension[]) => {
    if (!current()) return;
    entry.value = value;
    for (const l of entry.listeners) l();
  };
  listExtensions(key === "" ? null : key)
    .then(settle)
    .catch((e: unknown) => {
      if (!current()) return;
      recordOpError({ label: "Load extensions", message: e instanceof Error ? e.message : String(e) });
      settle(entry.value ?? []);
    });
}

function subscribe(key: string, listener: () => void): () => void {
  let entry = entries.get(key);
  if (!entry) {
    entry = { value: null, listeners: new Set(), seq: 0 };
    entries.set(key, entry);
    load(key, entry);
  }
  entry.listeners.add(listener);
  if (!offEvents) {
    const reloadAll = () => {
      for (const [k, e] of entries) load(k, e);
    };
    const offChanges = subscribeOxplowEvents((event) => {
      if (extensionsChanged(event as Record<string, unknown>)) reloadAll();
    });
    // A change while the daemon was unreachable sent no event: reload on
    // reconnect (tsk1030).
    const offReconnect = onRemoteReconnect(reloadAll);
    offEvents = () => {
      offChanges();
      offReconnect();
    };
  }
  const mine = entry;
  return () => {
    mine.listeners.delete(listener);
    if (mine.listeners.size === 0 && entries.get(key) === mine) entries.delete(key);
    if (entries.size === 0) {
      offEvents?.();
      offEvents = null;
    }
  };
}

/** The extensions loaded for `streamId` (`null` until the first load
 *  answers), re-rendering when they reload. */
export function useExtensions(streamId: string | null): Extension[] | null {
  const key = streamId ?? "";
  const sub = useCallback((listener: () => void) => subscribe(key, listener), [key]);
  return useSyncExternalStore(sub, () => entries.get(key)?.value ?? null);
}
