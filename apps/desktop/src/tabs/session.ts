/**
 * The navigation model of a thread's center tabs, as a browser keeps its
 * session history (`.context/pages-and-tabs.md` "Navigation model"). Pure
 * data and functions: the host holds a `Session` per thread, persists it
 * and draws from it — the strip, the bodies, Back/Forward, Up/Down, the
 * star and what agents read.
 *
 * - A **tab** is a slot (a stable name, never an id) with its entries and
 *   where it is among them.
 * - An **entry** is a page: its `ref`, whose id is a URI — what it shows
 *   (`?`, its identity) and how (`#`, its view) — its `historyState`
 *   (scroll, a draft, …), and `from`, the collection it was opened from.
 * - **One `navigate`**, as a browser has it: the same identity and mode in another
 *   view keeps the page and changes its view (a filter replaces the entry,
 *   a pick pushes one); another identity is another page.
 */

import type { SqlCell } from "../tauri-bridge/generated/bindings.js";
import type { TabRef } from "./tabState.js";
import { RENDERER_VIEW_KEY } from "../pages/renderers.js";
import { identityOf as uriIdentityOf, viewOf } from "../uri/uri.js";

/** One item of the collection a page was opened from. */
export interface OriginItem {
  ref: TabRef;
  label: string;
}

/** The collection a page was opened from: Up and Down step through it. */
export interface Origin {
  title?: string;
  /** The items as they were when the page opened; none for a live origin. */
  items: OriginItem[];
  index: number;
  /** The collection is a viz node's link column, run again whenever it is wanted
   *  (the host resolves `items` from it and finds `index` by the page). */
  live?: LiveOrigin;
}

export interface LiveOrigin {
  /** `<page>#<node>`: the viz node whose link column is the collection. */
  node: string;
  params: Record<string, SqlCell>;
  /** The column whose links are the pages. */
  column: string;
}

export interface Entry {
  ref: TabRef;
  /** This step's own memory (scroll, open sections, a draft), as a
   *  browser's `history.state`; persisted when it is JSON. */
  historyState?: unknown;
  from?: Origin;
}

export interface Tab {
  /** Stable for the tab's life, whatever it shows. */
  slot: string;
  entries: Entry[];
  /** The current entry's index. */
  at: number;
  /** It leads the strip and is always itself (an agent session's tab). */
  pinned?: true;
}

export interface Session {
  tabs: Tab[];
  /** The active tab's slot; null when none is chosen. */
  active: string | null;
  /** Slots, most recently used first. */
  mru: string[];
  /** The next slot's number. */
  next: number;
}

/** The most tabs a thread keeps open; pinned tabs don't count. */
export const MAX_PAGE_TABS = 15;

/** The slot of the tab showing the page `id` names now (its identity in its
 *  display mode, in any other view); null when none does. */
export function slotShowing(s: Session, id: string): string | null {
  const shown = showingOf(id);
  return s.tabs.find((t) => showingOf(current(t).ref.id) === shown)?.slot ?? null;
}

/** The active tab's current page; null when none is active. */
export function activeRef(s: Session | undefined): TabRef | null {
  const tab = s?.tabs.find((t) => t.slot === s.active);
  return tab ? current(tab).ref : null;
}

export function emptySession(): Session {
  return { tabs: [], active: null, mru: [], next: 1 };
}

export function current(tab: Tab): Entry {
  return tab.entries[tab.at]!;
}

/** A page's identity: its id without its view (`#`); an id that isn't one
 *  of oxplow's URIs is its own. */
export function identityOf(id: string): string {
  return uriIdentityOf(id) ?? id;
}

/** How a URI is drawn: the renderer its view names (`#oxplow.renderer`),
 *  or "" when it names none (the default, which only the daemon's renderer
 *  list can resolve — kept out of here so the session stays synchronous). An
 *  explicit default and an absent one are different modes, so writers that
 *  know the default leave the key out (Open With does). */
export function modeOf(id: string): string {
  return viewOf(id, RENDERER_VIEW_KEY) ?? "";
}

/** What a tab shows, for matching tabs: the identity in its display mode.
 *  A page (with its mode) shows once; the same resource in another mode is
 *  another tab. Anything about the resource itself (bookmarks, comments,
 *  backlinks) keeps `identityOf`. */
export function showingOf(id: string): string {
  const mode = modeOf(id);
  return mode ? `${identityOf(id)}\n${mode}` : identityOf(id);
}

function touched(mru: string[], slot: string): string[] {
  return [slot, ...mru.filter((s) => s !== slot)];
}

function withTab(s: Session, slot: string, f: (t: Tab) => Tab): Session {
  const i = s.tabs.findIndex((t) => t.slot === slot);
  if (i === -1) return s;
  const next = f(s.tabs[i]!);
  if (next === s.tabs[i]) return s;
  const tabs = [...s.tabs];
  tabs[i] = next;
  return { ...s, tabs };
}

/** Show `entry`: the tab already showing its page in its mode — in any
 *  view — now in `entry`'s (its ref refreshed, its state kept), else a new
 *  tab; the same page in another mode is another tab. The tab becomes
 *  active. */
export function open(s: Session, entry: Entry, opts: { pinned?: boolean } = {}): Session {
  const shown = showingOf(entry.ref.id);
  const there = s.tabs.find((t) => showingOf(current(t).ref.id) === shown);
  if (there) {
    const refreshed = withTab(s, there.slot, (t) => {
      const entries = [...t.entries];
      entries[t.at] = { ...entries[t.at]!, ref: entry.ref, ...(entry.from ? { from: entry.from } : {}) };
      return { ...t, entries };
    });
    return activate(refreshed, there.slot);
  }
  const slot = `t${s.next}`;
  const tab: Tab = { slot, entries: [entry], at: 0, ...(opts.pinned ? { pinned: true as const } : {}) };
  return { tabs: [...s.tabs, tab], active: slot, mru: touched(s.mru, slot), next: s.next + 1 };
}

/** Go to `entry` in `slot`, as a browser does: pushed after the current
 *  entry (what was forward is dropped), or in its place with `replace`.
 *  The same page in another view (`entry`'s identity and mode are the
 *  current entry's) is that page further on: it keeps the page's state (open
 *  sections, a draft) and its origin, and its body stays mounted; replaced,
 *  it is the entry rewritten (`history.replaceState`: what was forward
 *  stays). A change of mode (Open With) is a new page in this tab: no state
 *  carries. Another tab showing the page in the mode it arrives in closes,
 *  so a page shows once per mode; one in a different mode stays. */
export function navigate(s: Session, slot: string, entry: Entry, opts: { replace?: boolean } = {}): Session {
  const tab = s.tabs.find((t) => t.slot === slot);
  if (!tab) return s;
  const now = current(tab);
  if (now.ref.id === entry.ref.id) return s;
  const shown = showingOf(entry.ref.id);
  const samePage = showingOf(now.ref.id) === shown;
  const step: Entry = samePage
    ? {
        ...(now.from ? { from: now.from } : {}),
        ...(now.historyState !== undefined ? { historyState: now.historyState } : {}),
        ...entry,
      }
    : entry;
  if (samePage && opts.replace) {
    return withTab(s, slot, (t) => {
      const entries = [...t.entries];
      entries[t.at] = step;
      return { ...t, entries };
    });
  }
  const kept = opts.replace ? tab.entries.slice(0, tab.at) : tab.entries.slice(0, tab.at + 1);
  const moved: Tab = { ...tab, entries: [...kept, step], at: kept.length };
  const others = s.tabs.filter((t) => t.slot !== slot && showingOf(current(t).ref.id) === shown).map((t) => t.slot);
  const out = withTab(s, slot, () => moved);
  return others.reduce(close, out);
}

/** Keep `state` as the history state of `slot`'s current entry. */
export function setHistoryState(s: Session, slot: string, state: unknown): Session {
  return withTab(s, slot, (t) => {
    const entries = [...t.entries];
    entries[t.at] = { ...entries[t.at]!, historyState: state };
    return { ...t, entries };
  });
}

/** Merge `patch` into the current entry's history state when it is an object of
 *  keys (what pages keep through `useHistoryState`). A patch that changes
 *  no value returns `s` itself, so a repeated write costs no render. */
export function patchHistoryState(s: Session, slot: string, patch: Record<string, unknown>): Session {
  return withTab(s, slot, (t) => {
    const now = current(t);
    const had = now.historyState && typeof now.historyState === "object" && !Array.isArray(now.historyState) ? (now.historyState as Record<string, unknown>) : {};
    const same = Object.entries(patch).every(([k, v]) => JSON.stringify(had[k]) === JSON.stringify(v));
    if (same) return t;
    const entries = [...t.entries];
    entries[t.at] = { ...now, historyState: { ...had, ...patch } };
    return { ...t, entries };
  });
}

export function back(s: Session, slot: string): Session {
  return withTab(s, slot, (t) => (t.at > 0 ? { ...t, at: t.at - 1 } : t));
}

export function forward(s: Session, slot: string): Session {
  return withTab(s, slot, (t) => (t.at < t.entries.length - 1 ? { ...t, at: t.at + 1 } : t));
}

/** Show item `index` of the current entry's origin in its place: Up and
 *  Down step through a collection without touching history. A live origin
 *  steps through `items`, as the host resolved them. */
export function stepOrigin(s: Session, slot: string, index: number, items?: OriginItem[]): Session {
  return withTab(s, slot, (t) => {
    const now = current(t);
    const item = (items ?? now.from?.items)?.[index];
    if (!now.from || !item || index === now.from.index) return t;
    const entries = [...t.entries];
    entries[t.at] = { ref: item.ref, from: { ...now.from, index } };
    return { ...t, entries };
  });
}

/** Close `slot`. Closing the active tab leaves the most recent other one
 *  active, or none. */
export function close(s: Session, slot: string): Session {
  if (!s.tabs.some((t) => t.slot === slot)) return s;
  const mru = s.mru.filter((m) => m !== slot);
  const tabs = s.tabs.filter((t) => t.slot !== slot);
  const active = s.active === slot ? (mru.find((m) => tabs.some((t) => t.slot === m)) ?? null) : s.active;
  return { ...s, tabs, mru, active };
}

/** Each tab's current page, in strip order. */
export function refsOf(s: Session): TabRef[] {
  return s.tabs.map((t) => current(t).ref);
}

/** The strip shows `refs`, in that order: a tab already showing one keeps
 *  its slot and history, a new one opens (pinned when its ref is), and a
 *  tab showing none closes. The same session when nothing changes. */
export function setStrip(s: Session, refs: TabRef[]): Session {
  const now = refsOf(s);
  if (refs.length === now.length && refs.every((r, i) => r.id === now[i]!.id)) return s;
  let next = s.next;
  const tabs = refs.map((ref): Tab => {
    const there = s.tabs.find((t) => current(t).ref.id === ref.id);
    if (there) return there;
    const slot = `t${next++}`;
    return { slot, entries: [{ ref }], at: 0, ...(ref.pinned ? { pinned: true as const } : {}) };
  });
  const kept = (slot: string) => tabs.some((t) => t.slot === slot);
  const mru = s.mru.filter(kept);
  const active = s.active !== null && kept(s.active) ? s.active : (mru[0] ?? null);
  return { tabs, active, mru, next };
}

/** The page `slot` shows is gone (its record was deleted): go back,
 *  dropping its entry so Forward never leads to it; close the tab when
 *  there is nothing to go back to. */
export function dropCurrent(s: Session, slot: string): Session {
  const tab = s.tabs.find((t) => t.slot === slot);
  if (!tab) return s;
  if (tab.at === 0) return close(s, slot);
  return withTab(s, slot, (t) => ({ ...t, entries: t.entries.filter((_, i) => i !== t.at), at: t.at - 1 }));
}

/** The tabs in `order`; any it doesn't name follow, in their order. */
export function reorder(s: Session, order: string[]): Session {
  const named = order.flatMap((slot) => s.tabs.filter((t) => t.slot === slot));
  const rest = s.tabs.filter((t) => !order.includes(t.slot));
  return { ...s, tabs: [...named, ...rest] };
}

export function activate(s: Session, slot: string | null): Session {
  if (slot === null) return { ...s, active: null };
  if (!s.tabs.some((t) => t.slot === slot)) return s;
  return { ...s, active: slot, mru: touched(s.mru, slot) };
}

/** The slots to close so no more than `max` count against the cap: the
 *  least recently used first, never the active one, a pinned one or one
 *  `keep` protects (a file with unsaved changes). */
export function evictions(s: Session, max: number, keep: (t: Tab) => boolean = () => false): string[] {
  const counted = s.tabs.filter((t) => !t.pinned);
  const over = counted.length - max;
  if (over <= 0) return [];
  const rank = (slot: string) => {
    const i = s.mru.indexOf(slot);
    return i === -1 ? Number.MAX_SAFE_INTEGER : i;
  };
  return counted
    .filter((t) => t.slot !== s.active && !keep(t))
    .sort((x, y) => rank(y.slot) - rank(x.slot))
    .slice(0, over)
    .map((t) => t.slot);
}

/** One mounted page of a tab. */
export interface Body {
  /** Stable while it stays the same page: `<slot>/<identity>[\n<mode>]`. */
  key: string;
  /** The entry it draws. */
  entry: Entry;
  /** It is the tab's current entry. */
  current: boolean;
}

/**
 * The pages a tab keeps mounted: one per page identity and mode among its
 * entries (its id without its `#` view, but with its renderer), drawing the current entry when it is
 * that page's, else the nearest.
 * Every page is a function of its entry, so a change of view params (a
 * filter, a pick, Back between them) re-renders the page and never
 * remounts it; other pages stay mounted hidden, so Back shows them as they
 * were.
 */
export function bodies(tab: Tab): Body[] {
  const keyOf = (entry: Entry) => `${tab.slot}/${showingOf(entry.ref.id)}`;
  const nearest = new Map<string, number>();
  tab.entries.forEach((entry, i) => {
    const key = keyOf(entry);
    const seen = nearest.get(key);
    if (seen === undefined || Math.abs(i - tab.at) < Math.abs(seen - tab.at)) nearest.set(key, i);
  });
  return [...nearest.entries()]
    .sort((x, y) => x[1] - y[1])
    .map(([key, i]) => ({ key, entry: tab.entries[i]!, current: i === tab.at }));
}

/** Plain JSON: what survives storage as itself. */
function isJson(v: unknown): boolean {
  if (v === null || typeof v === "string" || typeof v === "boolean") return true;
  if (typeof v === "number") return Number.isFinite(v);
  if (Array.isArray(v)) return v.every(isJson);
  if (typeof v === "object" && Object.getPrototypeOf(v) === Object.prototype) return Object.values(v as object).every(isJson);
  return false;
}

function storable(entry: Entry): Entry {
  if (entry.historyState === undefined || isJson(entry.historyState)) return entry;
  const { historyState: _, ...rest } = entry;
  return rest;
}

export function serializeSession(s: Session): string {
  return JSON.stringify({ ...s, tabs: s.tabs.map((t) => ({ ...t, entries: t.entries.map(storable) })) });
}

/** A stored session, or null when the text isn't one. */
export function parseSession(text: string): Session | null {
  let v: unknown;
  try {
    v = JSON.parse(text);
  } catch {
    return null;
  }
  const s = v as Partial<Session> | null;
  if (!s || !Array.isArray(s.tabs) || !Array.isArray(s.mru) || typeof s.next !== "number") return null;
  const tabs = s.tabs.filter(
    (t): t is Tab =>
      !!t && typeof t.slot === "string" && Array.isArray(t.entries) && t.entries.length > 0 && Number.isInteger(t.at) && t.at >= 0 && t.at < t.entries.length,
  );
  const active = typeof s.active === "string" && tabs.some((t) => t.slot === s.active) ? s.active : null;
  return { tabs, active, mru: s.mru.filter((m) => tabs.some((t) => t.slot === m)), next: s.next };
}
