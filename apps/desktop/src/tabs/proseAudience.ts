// Per-page prose-audience selection. Agent-authored prose for the
// "big three" entities (wiki bodies, task descriptions, effort
// summaries) carries three variants — developer / executive / caveman
// (see crates/oxplow-domain/src/prose.rs). The page-level top-nav
// selector writes the choice here; the body renderer reads it. Scope is
// per-page (per-tab), keyed by `${threadId}::${tabId}` — the same
// pageKey PageNavigationContext exposes — so a wiki can be read in
// caveman while a task stays developer. Mirrors the bookmarks store:
// pure factory over an injected Storage, single localStorage blob.

export type ProseAudience = "developer" | "executive" | "caveman";

export const PROSE_AUDIENCES: ProseAudience[] = ["developer", "executive", "caveman"];
export const DEFAULT_PROSE_AUDIENCE: ProseAudience = "developer";

export function isProseAudience(v: unknown): v is ProseAudience {
  return v === "developer" || v === "executive" || v === "caveman";
}

export interface ProseAudienceApi {
  /** The selected audience for a page, or the default when unset /
   *  null / invalid. */
  get(pageKey: string | null): ProseAudience;
  /** Set the audience for a page. Writing the default removes the row
   *  (keep the blob small; default is implicit). */
  set(pageKey: string | null, audience: ProseAudience): void;
  /** Drop a page's row — called from closePageTab so closed tabs don't
   *  leak entries. */
  clear(pageKey: string): void;
  subscribe(fn: () => void): () => void;
}

const KEY = "oxplow.prose-audience.v1";

interface Storage {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
  removeItem(k: string): void;
}

function safeParse(raw: string | null): Record<string, ProseAudience> {
  if (!raw) return {};
  try {
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const out: Record<string, ProseAudience> = {};
    for (const [k, v] of Object.entries(parsed)) {
      if (isProseAudience(v)) out[k] = v;
    }
    return out;
  } catch {
    return {};
  }
}

/** Pure factory — pass an injected storage in tests. */
export function createProseAudienceStore(storage: Storage): ProseAudienceApi {
  const subs = new Set<() => void>();
  const notify = () => {
    for (const fn of subs) fn();
  };

  const readAll = () => safeParse(storage.getItem(KEY));
  const writeAll = (map: Record<string, ProseAudience>) => {
    if (Object.keys(map).length === 0) storage.removeItem(KEY);
    else storage.setItem(KEY, JSON.stringify(map));
    notify();
  };

  return {
    get(pageKey) {
      if (!pageKey) return DEFAULT_PROSE_AUDIENCE;
      const v = readAll()[pageKey];
      return isProseAudience(v) ? v : DEFAULT_PROSE_AUDIENCE;
    },
    set(pageKey, audience) {
      if (!pageKey) return;
      const map = readAll();
      if (audience === DEFAULT_PROSE_AUDIENCE) {
        if (!(pageKey in map)) return;
        delete map[pageKey];
      } else {
        if (map[pageKey] === audience) return;
        map[pageKey] = audience;
      }
      writeAll(map);
    },
    clear(pageKey) {
      const map = readAll();
      if (!(pageKey in map)) return;
      delete map[pageKey];
      writeAll(map);
    },
    subscribe(fn) {
      subs.add(fn);
      return () => {
        subs.delete(fn);
      };
    },
  };
}
