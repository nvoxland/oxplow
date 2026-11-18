/**
 * The context baskets in the app: what the person collected for agents (a
 * page from its ☰, text from a selection), in the selected thread's own
 * basket ("This Thread") or in a named basket of the project, read from
 * `v_context_item` and `v_context_basket` and written through the
 * `oxplow.context.*` commands. The next prompt to an agent in a thread
 * carries its own basket and every basket attached to it.
 * `.context/agent-model.md` → "The context basket".
 */
import { useCallback, useEffect, useState, useSyncExternalStore } from "react";

import { querySql } from "./api.js";
import { recordOpError } from "./opErrors.js";
import { NO_READS, unionReads, useRerunOnChange } from "./viz/rerun.js";
import { threadRowId } from "./modelIds.js";
import { personCommands } from "./personCommands.js";
import { dockStore } from "./components/Dock/dockStore.js";
import type { Reads, SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** What the thread's own basket is called. */
export const THREAD_BASKET = "This Thread";

// ---- the thread and stream the window shows ----------------------------

let selected: string | null = null;
let selectedStream: string | null = null;
const listeners = new Set<() => void>();

/** The thread on screen and its stream (`App` sets them as the selection
 *  changes). */
export function setSelectedThread(threadId: string | null, streamId: string | null = null): void {
  if (selected === threadId && selectedStream === streamId) return;
  selected = threadId;
  selectedStream = streamId;
  for (const l of [...listeners]) l();
}

export function selectedThreadId(): string | null {
  return selected;
}

/** The thread on screen, re-read as it changes. */
export function useSelectedThreadId(): string | null {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    () => selected,
  );
}

// ---- the items ----------------------------------------------------------

export interface BasketItem {
  id: number;
  kind: "page" | "text";
  sourceUri: string | null;
  label: string | null;
  text: string | null;
  /** The person's note on it: what they want done about it. */
  note: string | null;
  /** The named basket it is in; null: a thread's own. */
  basket: string | null;
}

/** A named basket and what is in it. */
export interface NamedBasket {
  id: number;
  name: string;
  /** The thread whose next prompt carries it (`thr<n>`), or null. */
  attachedTo: string | null;
  items: BasketItem[];
}

/** The selected thread's own basket and every named basket. */
export interface Baskets {
  own: BasketItem[];
  named: NamedBasket[];
}

export const NO_BASKETS: Baskets = { own: [], named: [] };

export const ITEMS_SQL = `
  SELECT id, kind, source_uri, label, text, note, basket
  FROM v_context_item
  WHERE thread_id = ?1 OR basket_id IS NOT NULL
  ORDER BY added_at, id`;

export const BASKETS_SQL = `
  SELECT id, name, attached_thread_id
  FROM v_context_basket
  ORDER BY name COLLATE NOCASE`;

function cells(result: SqlQueryResult) {
  const at = (row: SqlCell[], name: string) => row[result.columns.indexOf(name)] ?? null;
  const str = (v: SqlCell) => (v == null ? null : String(v));
  return { at, str };
}

export function itemsFromResult(result: SqlQueryResult): BasketItem[] {
  const { at, str } = cells(result);
  return result.rows.map((row) => ({
    id: Number(at(row, "id")),
    kind: at(row, "kind") === "text" ? "text" : "page",
    sourceUri: str(at(row, "source_uri")),
    label: str(at(row, "label")),
    text: str(at(row, "text")),
    note: str(at(row, "note")),
    basket: str(at(row, "basket")),
  }));
}

/** The baskets, from what `ITEMS_SQL` and `BASKETS_SQL` read. */
export function basketsFrom(items: readonly BasketItem[], baskets: SqlQueryResult): Baskets {
  const { at, str } = cells(baskets);
  return {
    own: items.filter((i) => i.basket == null),
    named: baskets.rows.map((row) => {
      const name = String(at(row, "name"));
      const attached = at(row, "attached_thread_id");
      return {
        id: Number(at(row, "id")),
        name,
        attachedTo: attached == null ? null : `thr${str(attached)}`,
        items: items.filter((i) => i.basket === name),
      };
    }),
  };
}

/** What `thread`'s next prompt will carry: its own items and those of every
 *  basket attached to it. */
export function nextPrompt(baskets: Baskets, thread: string | null): BasketItem[] {
  return [...baskets.own, ...baskets.named.filter((b) => thread != null && b.attachedTo === thread).flatMap((b) => b.items)];
}

/** The thread's own basket and every named basket, re-read as they change. */
export function useBaskets(threadId: string | null): Baskets {
  const [baskets, setBaskets] = useState<Baskets>(NO_BASKETS);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    let cancelled = false;
    const thread = threadId ? threadRowId(threadId) : 0;
    void Promise.all([querySql(ITEMS_SQL, [thread], 2_000), querySql(BASKETS_SQL, [], 500)]).then(
      ([items, named]) => {
        if (cancelled) return;
        setBaskets(basketsFrom(itemsFromResult(items), named));
        setReads(unionReads([items.reads, named.reads]));
      },
      (e: unknown) => recordOpError({ label: "Read the context baskets", message: e instanceof Error ? e.message : String(e) }),
    );
    return () => {
      cancelled = true;
    };
  }, [threadId]);
  useEffect(() => load(), [load]);
  useRerunOnChange(reads, () => void load());
  return baskets;
}

// ---- where an add goes ------------------------------------------------------

/** Where an add goes: the thread's own basket, or a named basket. */
export type Destination = { thread: string } | { basket: string };

/** The app preference that keeps the basket last added to: its name, or
 *  `THREAD_MARK` for This Thread. */
export const LAST_BASKET_PREF = "context.lastBasket";
export const THREAD_MARK = "@thread";

/** How `to` is remembered as the last basket. */
export function rememberedAs(to: Destination): string {
  return "basket" in to ? to.basket : THREAD_MARK;
}

/** The picker's first choice: the last basket added to while it is still
 *  there (This Thread only with a thread), else This Thread, else the first
 *  named basket; null: none, so a new one. */
export function defaultDestination(thread: string | null, names: readonly string[], last: string | null): Destination | null {
  if (last === THREAD_MARK && thread) return { thread };
  const kept = last == null ? undefined : names.find((n) => n.toLowerCase() === last.toLowerCase());
  if (kept) return { basket: kept };
  if (thread) return { thread };
  return names[0] ? { basket: names[0] } : null;
}

// ---- writes ---------------------------------------------------------------

/** What goes in a basket: a page (`uri`), or a piece of text from one. */
export interface ContextWhat {
  uri?: string | null;
  label?: string | null;
  text?: string | null;
  /** The lines around `text`, when `uri` has none to read back (a terminal). */
  around?: string | null;
}

/** What `oxplow.context.add` takes for a page, or for text (from a page),
 *  to `to`, collected in `stream`, with the person's note when they wrote one. */
export function addInput(to: Destination, stream: string | null, what: ContextWhat, note?: string | null) {
  const trimmed = note?.trim();
  return {
    ...to,
    ...(stream ? { stream } : {}),
    ...(what.uri ? { uri: what.uri } : {}),
    ...(what.label ? { label: what.label } : {}),
    ...(what.text ? { text: what.text } : {}),
    ...(what.text && what.around ? { around: what.around } : {}),
    ...(trimmed ? { note: trimmed } : {}),
  };
}

// ---- the add card -----------------------------------------------------------
//
// Every Add to Context offers a note and a basket before it adds:
// `addToContext` opens the Context dock panel, whose add card
// (`components/Context/ContextAddCard.tsx`) shows what is being added and
// adds it (`submitToContext`) or drops it. The draft — the note, the basket
// picked, a new basket's name — is kept here, so the dock closing on a press
// outside it loses nothing: opening the panel again shows the card as it was.

/** An add waiting on the person's note and basket. */
export interface PendingAdd {
  /** The thread on screen when it was asked for, if any. */
  thread: string | null;
  /** The stream it is collected in. */
  stream: string | null;
  what: ContextWhat;
  /** The note typed so far. */
  note: string;
  /** The basket picked: a name, `THREAD_MARK`, `NEW_MARK`; null until the
   *  card sets its default. */
  choice: string | null;
  /** A new basket's name, typed so far. */
  newName: string;
}

/** The add card's choice for a new basket. */
export const NEW_MARK = "@new";

let pending: PendingAdd | null = null;
const pendingListeners = new Set<() => void>();

function setPending(next: PendingAdd | null): void {
  pending = next;
  for (const l of pendingListeners) l();
}

/** The add the card is open for, or null. */
export function usePendingAdd(): PendingAdd | null {
  return useSyncExternalStore(
    (cb) => {
      pendingListeners.add(cb);
      return () => {
        pendingListeners.delete(cb);
      };
    },
    () => pending,
  );
}

/** Change the open add's draft. */
export function updatePendingAdd(patch: Partial<Pick<PendingAdd, "note" | "choice" | "newName">>): void {
  if (pending) setPending({ ...pending, ...patch });
}

/** Offer a note and a basket in the Context dock panel, then put a page
 *  (`uri`) or a piece of text in it. */
export async function addToContext(what: ContextWhat): Promise<void> {
  setPending({ thread: selected, stream: selectedStream, what, note: "", choice: null, newName: "" });
  dockStore.open("context");
}

/** Add the pending item to `to` with its note (none when blank) and, when
 *  it has none, `label`; the toast offers Undo. */
export async function submitToContext(add: PendingAdd, to: Destination, label?: string | null): Promise<void> {
  setPending(null);
  const what = add.what.label || !label ? add.what : { ...add.what, label };
  await personCommands.run("Add to Context", "oxplow.context.add", addInput(to, add.stream, what, add.note));
}

/** Drop the open add. */
export function cancelAddToContext(): void {
  setPending(null);
}

/** Set (or, blank, clear) an item's note. */
export async function setContextNote(item: number, note: string): Promise<void> {
  const trimmed = note.trim();
  await personCommands.run("Change Context Note", "oxplow.context.note", trimmed ? { item, note: trimmed } : { item });
}

export async function removeFromContext(item: number): Promise<void> {
  await personCommands.run("Remove from Context", "oxplow.context.remove", { item });
}

export async function moveContextItem(item: number, to: Destination): Promise<void> {
  await personCommands.run("Move to Basket", "oxplow.context.move", { item, ...to });
}

export async function clearContext(which: Destination): Promise<void> {
  await personCommands.run("Clear Context", "oxplow.context.clear", which);
}

export async function attachBasket(basket: string, thread: string): Promise<void> {
  await personCommands.run("Attach Basket", "oxplow.context.attach", { basket, thread });
}

export async function detachBasket(basket: string): Promise<void> {
  await personCommands.run("Detach Basket", "oxplow.context.detach", { basket });
}

export async function renameBasket(basket: string, name: string): Promise<void> {
  await personCommands.run("Rename Basket", "oxplow.context.rename_basket", { basket, name });
}

export async function deleteBasket(basket: string): Promise<void> {
  await personCommands.run("Delete Basket", "oxplow.context.delete_basket", { basket });
}
