/**
 * The context basket in the app: what the person collected for the
 * selected thread's agents (a page from its ☰, text from a selection),
 * read from `v_thread_context` and written through the
 * `oxplow.context.*` commands. The next prompt to an agent in the thread
 * carries what waits there. `.context/agent-model.md` → "The context basket".
 */
import { useCallback, useEffect, useState, useSyncExternalStore } from "react";

import { querySql } from "./api.js";
import { recordOpError } from "./opErrors.js";
import { NO_READS, useRerunOnChange } from "./viz/rerun.js";
import { threadRowId } from "./modelIds.js";
import { personCommands } from "./personCommands.js";
import type { Reads, SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

// ---- the thread the window shows --------------------------------------

let selected: string | null = null;
const listeners = new Set<() => void>();

/** The thread on screen (`App` sets it as the selection changes). */
export function setSelectedThread(threadId: string | null): void {
  if (selected === threadId) return;
  selected = threadId;
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
  delivered: boolean;
}

export const BASKET_SQL = `
  SELECT id, kind, source_uri, label, text, note, delivered_at
  FROM v_thread_context
  WHERE thread_id = ?1
  ORDER BY id`;

export function basketFromResult(result: SqlQueryResult): BasketItem[] {
  const at = (row: SqlCell[], name: string) => row[result.columns.indexOf(name)] ?? null;
  const str = (v: SqlCell) => (v == null ? null : String(v));
  return result.rows.map((row) => ({
    id: Number(at(row, "id")),
    kind: at(row, "kind") === "text" ? "text" : "page",
    sourceUri: str(at(row, "source_uri")),
    label: str(at(row, "label")),
    text: str(at(row, "text")),
    note: str(at(row, "note")),
    delivered: at(row, "delivered_at") != null,
  }));
}

/** What still waits for the thread's next prompt. */
export function waiting(items: readonly BasketItem[]): BasketItem[] {
  return items.filter((i) => !i.delivered);
}

/** The thread's basket (delivered items too), re-read as it changes. */
export function useBasket(threadId: string | null): BasketItem[] {
  const [items, setItems] = useState<BasketItem[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    if (!threadId) {
      setItems([]);
      return undefined;
    }
    let cancelled = false;
    void querySql(BASKET_SQL, [threadRowId(threadId)], 1_000).then(
      (res) => {
        if (cancelled) return;
        setItems(basketFromResult(res));
        setReads(res.reads);
      },
      (e: unknown) => recordOpError({ label: "Read the context basket", message: e instanceof Error ? e.message : String(e) }),
    );
    return () => {
      cancelled = true;
    };
  }, [threadId]);
  useEffect(() => load(), [load]);
  useRerunOnChange(reads, () => void load());
  return items;
}

// ---- writes ---------------------------------------------------------------

/** Why Add to Context isn't offered now, or null when it is. */
export function addDisabledReason(threadId: string | null): string | null {
  return threadId ? null : "Select a thread first";
}

/** What goes in the basket: a page (`uri`), or a piece of text from one. */
export interface ContextWhat {
  uri?: string | null;
  label?: string | null;
  text?: string | null;
}

/** What `oxplow.context.add` takes for a page, or for text (from a page),
 *  with the person's note when they wrote one. */
export function addInput(threadId: string, what: ContextWhat, note?: string | null) {
  const trimmed = note?.trim();
  return {
    thread: threadId,
    ...(what.uri ? { uri: what.uri } : {}),
    ...(what.label ? { label: what.label } : {}),
    ...(what.text ? { text: what.text } : {}),
    ...(trimmed ? { note: trimmed } : {}),
  };
}

// ---- the note prompt --------------------------------------------------------
//
// Every Add to Context offers a note before it adds: `addToContext` opens the
// one app-level `ContextNotePrompt` with what is being added, and that prompt
// adds it (`submitToContext`) or drops it.

/** An add waiting on the person's note. */
export interface PendingAdd {
  thread: string;
  what: ContextWhat;
}

let pending: PendingAdd | null = null;
const pendingListeners = new Set<() => void>();

function setPending(next: PendingAdd | null): void {
  pending = next;
  for (const l of pendingListeners) l();
}

/** The add the note prompt is open for, or null. */
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

/** Offer a note, then put a page (`uri`) or a piece of text in the selected
 *  thread's basket. Does nothing without a thread. */
export async function addToContext(what: ContextWhat): Promise<void> {
  const thread = selected;
  if (!thread) return;
  setPending({ thread, what });
}

/** Add the pending item with `note` (none when blank); the toast offers Undo. */
export async function submitToContext(add: PendingAdd, note: string): Promise<void> {
  setPending(null);
  await personCommands.run("Add to Context", "oxplow.context.add", addInput(add.thread, add.what, note));
}

/** Close the note prompt without adding. */
export function cancelAddToContext(): void {
  setPending(null);
}

/** Set (or, blank, clear) the note of an item still waiting. */
export async function setContextNote(item: number, note: string): Promise<void> {
  const trimmed = note.trim();
  await personCommands.run("Change Context Note", "oxplow.context.note", trimmed ? { item, note: trimmed } : { item });
}

export async function removeFromContext(item: number): Promise<void> {
  await personCommands.run("Remove from Context", "oxplow.context.remove", { item });
}

export async function clearContext(threadId: string): Promise<void> {
  await personCommands.run("Clear Context", "oxplow.context.clear", { thread: threadId });
}
