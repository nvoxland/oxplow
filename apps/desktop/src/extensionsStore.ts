import { useSyncExternalStore } from "react";

import { listExtensions, onRemoteReconnect, subscribeOxplowEvents, type Extension } from "./api.js";
import { recordOpError } from "./components/opErrorsStore.js";
import { extensionsChanged } from "./lens/lensRerun.js";

/**
 * The loaded extensions — the main worktree's, the same in every stream —
 * shared by every component that reads them: one `listExtensions` and one
 * event subscription, however many menus, slots and decorators are
 * mounted. The listing lives while something reads it and is dropped with
 * its last reader, so a later mount loads afresh. It reloads when the
 * extensions' definitions change (`extensionsChanged`).
 */
let value: Extension[] | null = null;
const listeners = new Set<() => void>();
/** The latest load; an older answer is dropped. */
let seq = 0;
let offEvents: (() => void) | null = null;

function load() {
  const mine = ++seq;
  const settle = (next: Extension[]) => {
    if (mine !== seq || listeners.size === 0) return;
    value = next;
    for (const l of listeners) l();
  };
  listExtensions()
    .then(settle)
    .catch((e: unknown) => {
      if (mine !== seq || listeners.size === 0) return;
      recordOpError({ label: "Load extensions", message: e instanceof Error ? e.message : String(e) });
      settle(value ?? []);
    });
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  if (!offEvents) {
    load();
    const offChanges = subscribeOxplowEvents((event) => {
      if (extensionsChanged(event as Record<string, unknown>)) load();
    });
    // A change while the daemon was unreachable sent no event: reload on
    // reconnect (tsk1030).
    const offReconnect = onRemoteReconnect(load);
    offEvents = () => {
      offChanges();
      offReconnect();
    };
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      offEvents?.();
      offEvents = null;
      value = null;
      seq++;
    }
  };
}

/** The extensions the app shows (`null` until the first load answers),
 *  re-rendering when they reload. */
export function useExtensions(): Extension[] | null {
  return useSyncExternalStore(subscribe, () => value);
}
