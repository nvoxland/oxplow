/**
 * What the person's oxplow remembers for them outside any one tab
 * (`ui_preference`, `.context/pages-and-tabs.md` "Page state"):
 *
 * - **page memory** — this page, for this person, shared by every tab of it
 *   (`usePageMemory`): the Git dashboard's merge or rebase, a terminal
 *   page's terminals;
 * - **app preferences** — the whole app (`useAppPreference`): the rail's
 *   width, which Quick Open sections are open;
 * - a panel's memory (`usePanelMemory`): its choice params.
 *
 * A tab's own view is its **view params** and its own memory its **history
 * state**; neither is here. Values are JSON. One store loads everything
 * once and writes a change after a pause, per key; a value equal to what's
 * there writes nothing.
 */

import { createContext, useCallback, useContext, useState, useSyncExternalStore } from "react";

import { getUiPreferences, setUiPreference } from "../api.js";
import { logUi } from "../logger.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { refOf } from "../refs/ref.js";
import { identityOf } from "../tabs/session.js";
import type { TabRef } from "../tabs/tabState.js";
import type { UiPreference } from "../tauri-bridge/generated/bindings.js";

/** Where remembered values are read from and written to. */
export interface RememberedBackend {
  load(): Promise<UiPreference[]>;
  /** Keep `valueJson` under `scope` and `key`; null forgets it. */
  save(scope: string, key: string, valueJson: string | null): Promise<void>;
}

export interface RememberedStore {
  /** Resolves once what was kept is loaded. */
  ready: Promise<void>;
  get(scope: string, key: string): unknown;
  /** Remember `value`; `undefined` forgets it. */
  set(scope: string, key: string, value: unknown): void;
  subscribe(listener: () => void): () => void;
  /** Changes on every change: what `useSyncExternalStore` compares. */
  version(): number;
}

const at = (scope: string, key: string) => `${scope}\u0000${key}`;

export function createRememberedStore(backend: RememberedBackend, debounceMs = 300): RememberedStore {
  const values = new Map<string, unknown>();
  const listeners = new Set<() => void>();
  const pending = new Map<string, ReturnType<typeof setTimeout>>();
  let version = 0;
  const changed = () => {
    version += 1;
    for (const listener of Array.from(listeners)) listener();
  };
  const ready = backend
    .load()
    .then((rows) => {
      for (const row of rows) {
        // A value kept before this load finished wins: it is newer.
        if (values.has(at(row.scope, row.key))) continue;
        try {
          values.set(at(row.scope, row.key), JSON.parse(row.valueJson));
        } catch {
          // Not JSON: as if never kept.
        }
      }
      changed();
    })
    .catch((e: unknown) => logUi("warn", "couldn't read what was remembered", { error: String(e) }));
  return {
    ready,
    get: (scope, key) => values.get(at(scope, key)),
    set: (scope, key, value) => {
      const id = at(scope, key);
      if (JSON.stringify(values.get(id)) === JSON.stringify(value)) return;
      if (value === undefined) values.delete(id);
      else values.set(id, value);
      changed();
      clearTimeout(pending.get(id));
      pending.set(
        id,
        setTimeout(() => {
          pending.delete(id);
          const now = values.get(id);
          void backend
            .save(scope, key, now === undefined ? null : JSON.stringify(now))
            .catch((e: unknown) => logUi("warn", "couldn't remember a value", { scope, key, error: String(e) }));
        }, debounceMs),
      );
    },
    subscribe: (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    version: () => version,
  };
}

let shared: RememberedStore | null = null;
/** The app's store, made on first use. */
function appStore(): RememberedStore {
  shared ??= createRememberedStore({ load: getUiPreferences, save: setUiPreference });
  return shared;
}

/** The store hooks read; the app's own unless a provider (a test) says. */
export const RememberedContext = createContext<RememberedStore | null>(null);

/** The store the hooks read, for a component that needs many panels' values at once. */
export function useRememberedStore(): RememberedStore {
  const store = useContext(RememberedContext) ?? appStore();
  useSyncExternalStore(store.subscribe, store.version);
  return store;
}

/** The page-memory scope of a page: its own ref without its view params
 *  (`page:tasks`, `page:acme/pulls`, `lens:x/y`), or `kind:<kind>` for an
 *  entity page (every file page shares one). */
export function pageMemoryScope(ref: TabRef): string {
  return ref.kind === "page" || ref.kind === "lens" ? identityOf(ref.id) : `kind:${ref.kind}`;
}

/** One remembered value under `scope` (null: none — kept in the caller). */
function useRemembered<T>(scope: string | null, key: string, initial: T): [T, (value: T) => void] {
  const store = useContext(RememberedContext) ?? appStore();
  useSyncExternalStore(store.subscribe, store.version);
  const [own, setOwn] = useState<T>(initial);
  const kept = scope === null ? undefined : store.get(scope, key);
  const value = scope === null ? own : kept === undefined ? initial : (kept as T);
  const set = useCallback(
    (next: T) => {
      if (scope === null) setOwn(next);
      else store.set(scope, key, next);
    },
    [store, scope, key],
  );
  return [value, set];
}

/** **Page memory**: one value this page remembers for the person, the same
 *  in every tab of it, kept across restarts. Outside a tab it is the
 *  component's own. */
export function usePageMemory<T>(key: string, initial: T): [T, (value: T) => void] {
  const ref = useOptionalPageNavigation()?.ref;
  return useRemembered(ref ? pageMemoryScope(ref) : null, key, initial);
}

/** **App preference**: one value the whole app remembers for the person. */
export function useAppPreference<T>(key: string, initial: T): [T, (value: T) => void] {
  return useRemembered("app", key, initial);
}

/** A left-nav panel's memory: one value it remembers for the person. */
export function usePanelMemory<T>(panel: string, key: string, initial: T): [T, (value: T) => void] {
  return useRemembered(panelScope(panel), key, initial);
}

/** The scope a panel's memory is kept under. */
export const panelScope = (panel: string) => refOf("panel", panel);
