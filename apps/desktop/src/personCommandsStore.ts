import { useSyncExternalStore } from "react";

import { listPersonCommands, onRemoteReconnect, subscribeOxplowEvents } from "./api.js";
import { recordOpError } from "./components/opErrorsStore.js";
import { extensionsChanged } from "./lens/lensRerun.js";
import type { CommandSpec } from "./tauri-bridge/generated/bindings.js";

/**
 * The commands a person is offered (`list_person_commands`), shared by
 * every reader — search, keyboard shortcuts — as one listing and one event
 * subscription, the way `extensionsStore` shares the extensions. It reloads
 * when what's offered may have changed: an extension's commands came or
 * went, or the config (a capability switched) changed — and on reconnect.
 */
let value: CommandSpec[] | null = null;
const listeners = new Set<() => void>();
let seq = 0;
let off: (() => void) | null = null;

function load() {
  const mine = ++seq;
  listPersonCommands()
    .then((next) => {
      if (mine !== seq || listeners.size === 0) return;
      value = next;
      for (const l of listeners) l();
    })
    .catch((e: unknown) => {
      if (mine !== seq) return;
      recordOpError({ label: "Load commands", message: e instanceof Error ? e.message : String(e) });
    });
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  if (!off) {
    load();
    const offEvents = subscribeOxplowEvents((event) => {
      if (extensionsChanged(event as Record<string, unknown>)) load();
    });
    const offReconnect = onRemoteReconnect(load);
    off = () => {
      offEvents();
      offReconnect();
    };
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      off?.();
      off = null;
      value = null;
      seq++;
    }
  };
}

/** The commands a person is offered (`[]` until the first load answers). */
export function usePersonCommands(): CommandSpec[] {
  return useSyncExternalStore(subscribe, () => value) ?? EMPTY;
}

const EMPTY: CommandSpec[] = [];
