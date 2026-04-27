/**
 * Per-thread tab state. Each thread has its own list of open tabs, an
 * active tab id, and (subscribers permitting) emits change notifications.
 *
 * Tab state is held in memory; it does NOT persist across app restarts in
 * v1. Threads exist for the lifetime of the daemon's session, and the
 * agent terminal — the always-present per-thread tab — already has its
 * own server-side resume mechanism.
 */

/** Stable identifier for the kind of content a tab renders. */
export type PageKind =
  | "agent"
  | "file"
  | "diff"
  | "note"
  | "work-item"
  | "finding"
  | "plan-work"
  | "done-work"
  | "backlog"
  | "archived"
  | "notes-index"
  | "files"
  | "code-quality"
  | "local-history"
  | "git-history"
  | "git-dashboard"
  | "git-commit"
  | "uncommitted-changes"
  | "hook-events"
  | "subsystem-docs"
  | "settings"
  | "dashboard"
  | "new-stream"
  | "new-work-item"
  | "stream-settings"
  | "thread-settings"
  | "closed-threads";

/** Reference to a tab. `id` must be unique across page kinds within a thread. */
export interface TabRef {
  id: string;
  kind: PageKind;
  /** Page-kind-specific data — file path, work item id, dashboard variant, etc. */
  payload: unknown;
}

/**
 * Per-tab navigation slot. Tabs in the rail/strip carry a stable
 * `slotId` that survives in-tab navigation; the *current* page they
 * render is `current`, with `back`/`forward` stacks for browser-style
 * history (most recent first). Pure data — operate on these via the
 * helpers below (`createSlot`, `navigateInSlot`, `slotGoBack`,
 * `slotGoForward`).
 */
export interface PageSlot {
  slotId: string;
  current: TabRef;
  back: TabRef[];
  forward: TabRef[];
}

let slotCounter = 0;

/** Reset the auto-incrementing slot id counter. Tests only. */
export function resetSlotCounter(): void {
  slotCounter = 0;
}

/** Generate a stable, unique slot id. */
export function nextSlotId(): string {
  slotCounter += 1;
  return `slot:${slotCounter}`;
}

/** Build a fresh slot anchored at `ref`. */
export function createSlot(ref: TabRef, slotId?: string): PageSlot {
  return { slotId: slotId ?? nextSlotId(), current: ref, back: [], forward: [] };
}

/**
 * Navigate within a slot to a new ref. Pushes the current page onto
 * `back`, clears `forward`. If `ref` is identical (by id) to the
 * current page, returns the slot unchanged. If `ref` matches the top
 * of `back`, treat as a back-navigation instead of a push (so a user
 * navigating B → A while A is at the top of back doesn't double-stack).
 */
export function navigateInSlot(slot: PageSlot, ref: TabRef): PageSlot {
  if (slot.current.id === ref.id) return slot;
  const backTop = slot.back[slot.back.length - 1];
  if (backTop && backTop.id === ref.id) {
    return slotGoBack(slot) ?? slot;
  }
  return {
    slotId: slot.slotId,
    current: ref,
    back: [...slot.back, slot.current],
    forward: [],
  };
}

/** Pop the top of `back` into `current`, push old current onto `forward`. */
export function slotGoBack(slot: PageSlot): PageSlot | null {
  if (slot.back.length === 0) return null;
  const next = slot.back[slot.back.length - 1]!;
  return {
    slotId: slot.slotId,
    current: next,
    back: slot.back.slice(0, -1),
    forward: [...slot.forward, slot.current],
  };
}

/** Pop the top of `forward` into `current`, push old current onto `back`. */
export function slotGoForward(slot: PageSlot): PageSlot | null {
  if (slot.forward.length === 0) return null;
  const next = slot.forward[slot.forward.length - 1]!;
  return {
    slotId: slot.slotId,
    current: next,
    back: [...slot.back, slot.current],
    forward: slot.forward.slice(0, -1),
  };
}

export interface ThreadTabState {
  tabs: TabRef[];
  activeId: string | null;
}

export interface OpenTabOptions {
  /** When true, replace the active tab in place rather than adding a new one. */
  replace?: boolean;
}

export interface TabStore {
  getThreadState(threadId: string): ThreadTabState;
  openTab(threadId: string, ref: TabRef, opts?: OpenTabOptions): void;
  /** Add a tab if missing without changing the active tab. */
  ensureTab(threadId: string, ref: TabRef): void;
  activate(threadId: string, tabId: string): void;
  closeTab(threadId: string, tabId: string): void;
  /** Subscribe to changes for one thread. Returns unsubscribe. */
  subscribe(threadId: string, fn: () => void): () => void;
}

const EMPTY: ThreadTabState = Object.freeze({ tabs: [], activeId: null }) as ThreadTabState;

export function createTabStore(): TabStore {
  const states = new Map<string, ThreadTabState>();
  const subscribers = new Map<string, Set<() => void>>();

  function readState(threadId: string): ThreadTabState {
    return states.get(threadId) ?? EMPTY;
  }

  function writeState(threadId: string, next: ThreadTabState): void {
    states.set(threadId, next);
    const subs = subscribers.get(threadId);
    if (subs) {
      for (const fn of subs) fn();
    }
  }

  return {
    getThreadState(threadId: string): ThreadTabState {
      return readState(threadId);
    },
    openTab(threadId, ref, opts) {
      const state = readState(threadId);
      const existingIdx = state.tabs.findIndex((t) => t.id === ref.id);

      if (opts?.replace && state.activeId) {
        const activeIdx = state.tabs.findIndex((t) => t.id === state.activeId);
        if (activeIdx >= 0) {
          // If the new ref already exists elsewhere, drop the old occurrence
          // and keep order stable around the active slot.
          const tabs = state.tabs.slice();
          tabs.splice(activeIdx, 1, ref);
          if (existingIdx >= 0 && existingIdx !== activeIdx) {
            // Remove the old duplicate, accounting for index shifts.
            const dupIdx = existingIdx > activeIdx ? existingIdx : existingIdx;
            tabs.splice(dupIdx, 1);
          }
          writeState(threadId, { tabs, activeId: ref.id });
          return;
        }
      }

      if (existingIdx >= 0) {
        if (state.activeId === ref.id) return;
        writeState(threadId, { tabs: state.tabs, activeId: ref.id });
        return;
      }
      writeState(threadId, {
        tabs: [...state.tabs, ref],
        activeId: ref.id,
      });
    },
    ensureTab(threadId, ref) {
      const state = readState(threadId);
      if (state.tabs.some((t) => t.id === ref.id)) return;
      writeState(threadId, {
        tabs: [...state.tabs, ref],
        activeId: state.activeId ?? ref.id,
      });
    },
    activate(threadId, tabId) {
      const state = readState(threadId);
      if (!state.tabs.some((t) => t.id === tabId)) return;
      if (state.activeId === tabId) return;
      writeState(threadId, { tabs: state.tabs, activeId: tabId });
    },
    closeTab(threadId, tabId) {
      const state = readState(threadId);
      const idx = state.tabs.findIndex((t) => t.id === tabId);
      if (idx < 0) return;
      const tabs = [...state.tabs.slice(0, idx), ...state.tabs.slice(idx + 1)];
      let activeId = state.activeId;
      if (activeId === tabId) {
        // Focus the previous tab; if closing the first, focus the new first;
        // if no tabs remain, null.
        if (tabs.length === 0) activeId = null;
        else if (idx === 0) activeId = tabs[0]?.id ?? null;
        else activeId = tabs[idx - 1]?.id ?? null;
      }
      writeState(threadId, { tabs, activeId });
    },
    subscribe(threadId, fn) {
      let set = subscribers.get(threadId);
      if (!set) {
        set = new Set();
        subscribers.set(threadId, set);
      }
      set.add(fn);
      return () => {
        set?.delete(fn);
      };
    },
  };
}
