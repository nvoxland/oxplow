import type { TabRef } from "./tabState.js";

export type BookmarkScope = "thread" | "stream" | "global";

export interface Bookmark {
  ref: TabRef;
  /** Optional human-friendly label captured at bookmark time. The page
   *  ref id is opaque ("git-dashboard", "wi:wi-…"); this is what the
   *  rail HUD row displays. */
  label?: string;
  scope: BookmarkScope;
  addedAt: number;
}

export interface BookmarksApi {
  /** Merged set across all scopes (thread ∪ stream ∪ global), deduped
   *  by ref id, scope precedence: thread > stream > global. Sorted by
   *  `addedAt` descending. */
  bookmarks(threadId: string | null, streamId: string | null): Bookmark[];
  isBookmarked(threadId: string | null, streamId: string | null, refId: string): boolean;
  scopesFor(threadId: string | null, streamId: string | null, refId: string): BookmarkScope[];
  add(scope: BookmarkScope, threadId: string | null, streamId: string | null, ref: TabRef, label?: string): void;
  remove(scope: BookmarkScope, threadId: string | null, streamId: string | null, refId: string): void;
  /** The scope the bookmark button defaults to when toggled; persisted. */
  lastScope(): BookmarkScope;
  setLastScope(scope: BookmarkScope): void;
  subscribe(fn: () => void): () => void;
}

const KEY_GLOBAL = "oxplow.bookmarks.v1.global";
const KEY_LAST_SCOPE = "oxplow.bookmarks.v1.lastScope";
const keyStream = (id: string) => `oxplow.bookmarks.v1.stream.${id}`;
const keyThread = (id: string) => `oxplow.bookmarks.v1.thread.${id}`;

interface Storage {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
  removeItem(k: string): void;
}

function safeParse(raw: string | null): Bookmark[] {
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((b) => b && typeof b === "object" && b.ref && typeof b.scope === "string");
  } catch {
    return [];
  }
}

/** Pure factory — pass an injected storage in tests. */
export function createBookmarksStore(storage: Storage): BookmarksApi {
  const subs = new Set<() => void>();
  const notify = () => { for (const fn of subs) fn(); };

  function readScope(scope: BookmarkScope, threadId: string | null, streamId: string | null): Bookmark[] {
    if (scope === "global") return safeParse(storage.getItem(KEY_GLOBAL));
    if (scope === "stream") return streamId ? safeParse(storage.getItem(keyStream(streamId))) : [];
    return threadId ? safeParse(storage.getItem(keyThread(threadId))) : [];
  }

  function writeScope(scope: BookmarkScope, threadId: string | null, streamId: string | null, list: Bookmark[]): void {
    const key = scope === "global"
      ? KEY_GLOBAL
      : scope === "stream"
        ? (streamId ? keyStream(streamId) : null)
        : (threadId ? keyThread(threadId) : null);
    if (!key) return;
    if (list.length === 0) storage.removeItem(key);
    else storage.setItem(key, JSON.stringify(list));
    notify();
  }

  return {
    bookmarks(threadId, streamId) {
      const t = readScope("thread", threadId, streamId);
      const s = readScope("stream", threadId, streamId);
      const g = readScope("global", threadId, streamId);
      const seen = new Set<string>();
      const merged: Bookmark[] = [];
      for (const list of [t, s, g]) {
        for (const b of list) {
          if (seen.has(b.ref.id)) continue;
          seen.add(b.ref.id);
          merged.push(b);
        }
      }
      merged.sort((a, b) => b.addedAt - a.addedAt);
      return merged;
    },
    isBookmarked(threadId, streamId, refId) {
      return this.scopesFor(threadId, streamId, refId).length > 0;
    },
    scopesFor(threadId, streamId, refId) {
      const out: BookmarkScope[] = [];
      if (readScope("thread", threadId, streamId).some((b) => b.ref.id === refId)) out.push("thread");
      if (readScope("stream", threadId, streamId).some((b) => b.ref.id === refId)) out.push("stream");
      if (readScope("global", threadId, streamId).some((b) => b.ref.id === refId)) out.push("global");
      return out;
    },
    add(scope, threadId, streamId, ref, label) {
      const list = readScope(scope, threadId, streamId);
      if (list.some((b) => b.ref.id === ref.id)) return;
      list.unshift({ ref, label, scope, addedAt: Date.now() });
      writeScope(scope, threadId, streamId, list);
    },
    remove(scope, threadId, streamId, refId) {
      const list = readScope(scope, threadId, streamId);
      const next = list.filter((b) => b.ref.id !== refId);
      if (next.length === list.length) return;
      writeScope(scope, threadId, streamId, next);
    },
    lastScope(): BookmarkScope {
      const raw = storage.getItem(KEY_LAST_SCOPE);
      if (raw === "thread" || raw === "stream" || raw === "global") return raw;
      return "thread";
    },
    setLastScope(scope) {
      storage.setItem(KEY_LAST_SCOPE, scope);
      notify();
    },
    subscribe(fn) {
      subs.add(fn);
      return () => { subs.delete(fn); };
    },
  };
}
