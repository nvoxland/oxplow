// Pure helpers for the per-thread page-tab cap + LRU eviction. Kept
// React-free so the ranking math is unit-tested without mounting App.
//
// The cap applies to *page tabs* (the entries in `threadPageTabs`); a
// pinned tab (an agent session's) is never counted or evicted.

/** Maximum number of open, unpinned page tabs per thread. */
export const MAX_PAGE_TABS = 15;

/** Move `id` to the front of `mru` (most-recently-used first), deduped. */
export function touchMru(mru: string[], id: string): string[] {
  if (mru[0] === id) return mru;
  const seen = new Set([id]);
  const tail = mru.filter((x) => (seen.has(x) ? false : (seen.add(x), true)));
  return [id, ...tail];
}

/** Remove `id` from `mru`. Returns the same ref when `id` is absent. */
export function dropFromMru(mru: string[], id: string): string[] {
  if (!mru.includes(id)) return mru;
  return mru.filter((x) => x !== id);
}

/**
 * Choose which open page tabs to evict so `tabIds` shrinks to at most
 * `max`. Eviction is least-recently-used first, derived from `mru`
 * (most-recently-used first):
 *
 * - Tabs absent from `mru` (never activated) are treated as the oldest and
 *   evicted before any activated tab.
 * - `protect` ids are never evicted (the active tab, dirty file tabs).
 * - `pinned` ids (agent sessions' tabs) are neither evicted nor counted.
 * - Stale `mru` ids that are no longer open are ignored.
 *
 * Best-effort: if protected tabs alone exceed `max`, returns fewer victims
 * than the overflow — the cap is a soft ceiling, never a reason to drop
 * unsaved work.
 */
export function selectLruEvictions(
  tabIds: string[],
  mru: string[],
  opts: { max: number; protect: Iterable<string>; pinned?: Iterable<string> },
): string[] {
  const pinned = new Set(opts.pinned ?? []);
  const counted = tabIds.filter((id) => !pinned.has(id));
  const overBy = counted.length - opts.max;
  if (overBy <= 0) return [];
  const protectedSet = new Set([...opts.protect, ...pinned]);
  const live = new Set(counted);
  const inMru = new Set(mru);
  // LRU-first ranking: never-activated tabs (oldest) first, then activated
  // tabs from least- to most-recently-used (mru reversed).
  const neverActivated = counted.filter((id) => !inMru.has(id));
  const byLru = [...mru].reverse().filter((id) => live.has(id));
  const ranked = [...neverActivated, ...byLru];
  const victims: string[] = [];
  for (const id of ranked) {
    if (victims.length >= overBy) break;
    if (protectedSet.has(id)) continue;
    victims.push(id);
  }
  return victims;
}
