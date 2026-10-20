/**
 * One piece of a page's state as easily as `useState`, but kept where it
 * belongs (`.context/pages-and-tabs.md` "Page state"):
 *
 * - `useViewParam(key, initial)` — a **view param**, a filter: what the
 *   page shows, in its tab's address. A change replaces the tab's entry; a
 *   value at its default leaves the address. With `{ push: true }` a change
 *   is a step Back returns from ("View all").
 * - `useViewSelection(key)` — a view param that is a **pick**: setting one
 *   pushes an entry, so Back un-selects; clearing replaces.
 * - `useViewList(key)` — a view param holding several values.
 * - `useCollapsedSet(key)` — which groups are open or closed, in this
 *   step's **history state**.
 * - `useDraft(key, initial)` — an unsaved form, in history state.
 *
 * A built-in page declares its view params' keys (`BUILT_IN_VIEW`,
 * `pages/pageIdentity.ts`); the host refuses one it doesn't. Outside a tab
 * each is the page's own state. A cursor moving over a list before
 * anything is picked is not a selection: it stays the list's own.
 */

import { useCallback, useState } from "react";

import type { SqlCell } from "../tauri-bridge/generated/bindings.js";
import { useOptionalPageNavigation } from "./PageNavigationContext.js";
import { useHistoryState } from "./useHistoryState.js";

/** `raw` in `initial`'s type; `initial` when it isn't one. */
function typed<T extends SqlCell>(raw: SqlCell | undefined, initial: T): T {
  if (raw === undefined || raw === null) return initial;
  if (typeof initial === "number") {
    const n = typeof raw === "number" ? raw : Number(raw);
    return (Number.isFinite(n) ? n : initial) as T;
  }
  if (typeof initial === "boolean") {
    if (raw === true || raw === "true" || raw === 1 || raw === "1") return true as T;
    if (raw === false || raw === "false" || raw === 0 || raw === "0") return false as T;
    return initial;
  }
  return (typeof initial === "string" ? String(raw) : raw) as T;
}

/** One view param, written as `push` says; local outside a tab. */
function useViewValue<T extends SqlCell>(key: string, initial: T, push: (next: T) => boolean): [T, (next: T) => void] {
  const nav = useOptionalPageNavigation();
  const [own, setOwn] = useState<T>(initial);
  const inTab = !!nav?.patchViewParams;
  const value = inTab ? typed(nav!.viewParams?.[key], initial) : own;
  const set = useCallback(
    (next: T) => {
      if (!nav?.patchViewParams) return setOwn(next);
      if (next === value) return;
      nav.patchViewParams({ [key]: next === initial ? undefined : next }, push(next) ? { push: true } : undefined);
    },
    // `push` is a fixed rule per hook.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [nav, key, value, initial],
  );
  return [value, set];
}

const never = () => false;
const always = () => true;

/** A **view param** that filters what the page shows (a range, a search,
 *  a mode): a change replaces the tab's entry, or with `push` adds one
 *  (a step Back returns from: "View all"). */
export function useViewParam<T extends SqlCell>(key: string, initial: T, opts?: { push?: boolean }): [Widen<T>, (next: Widen<T>) => void] {
  return useViewValue<Widen<T>>(key, initial as Widen<T>, opts?.push ? always : never);
}

/** `""` as `string`, `false` as `boolean`: a param's type, not its default's. */
type Widen<T> = T extends string ? string : T extends number ? number : T extends boolean ? boolean : T;

/** A **view param** that is a pick (a row, a commit, a model): picking
 *  pushes an entry, so Back un-selects; clearing replaces. */
export function useViewSelection(key: string): [string | null, (next: string | null) => void] {
  return useViewValue<string | null>(key, null, (next) => next !== null);
}

/** A **view param** holding several values (authors, states), comma-joined
 *  in the address. */
export function useViewList(key: string): [string[], (next: readonly string[]) => void] {
  const [joined, setJoined] = useViewParam(key, "");
  const values = joined.split(",").filter(Boolean);
  const set = useCallback((next: readonly string[]) => setJoined(next.join(",")), [setJoined]);
  return [values, set];
}

/** Which ids (groups, tree nodes, rows) are in a set — open or closed, as
 *  the caller means it — kept in this step's **history state**. */
export function useCollapsedSet(key: string): { has(id: string): boolean; toggle(id: string): void; set(ids: readonly string[]): void } {
  const [ids, setIds] = useHistoryState<string[]>(key, []);
  return {
    has: (id) => ids.includes(id),
    toggle: (id) => setIds(ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id]),
    set: (next) => setIds([...next]),
  };
}

/** An unsaved form's values, kept in this step's **history state**: Back
 *  and a restart bring them back; `clear` (on submit) starts afresh. */
export function useDraft<T>(key: string, initial: T): [T, (next: T) => void, () => void] {
  const [draft, setDraft] = useHistoryState<T>(key, initial);
  const clear = useCallback(() => setDraft(initial), [setDraft, initial]);
  return [draft, setDraft, clear];
}
