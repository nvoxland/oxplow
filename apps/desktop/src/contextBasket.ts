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
import { NO_READS, useRerunOnChange } from "./lens/lensRerun.js";
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
  delivered: boolean;
}

export const BASKET_SQL = `
  SELECT id, kind, source_uri, label, text, delivered_at
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

/** What `oxplow.context.add` takes for a page, or for text (from a page). */
export function addInput(threadId: string, what: { uri?: string | null; label?: string | null; text?: string | null }) {
  return {
    thread: threadId,
    ...(what.uri ? { uri: what.uri } : {}),
    ...(what.label ? { label: what.label } : {}),
    ...(what.text ? { text: what.text } : {}),
  };
}

/** Put a page (`uri`) or a piece of text in the selected thread's basket;
 *  the toast offers Undo. Does nothing without a thread. */
export async function addToContext(what: { uri?: string | null; label?: string | null; text?: string | null }): Promise<void> {
  const thread = selected;
  if (!thread) return;
  await personCommands.run("Add to Context", "oxplow.context.add", addInput(thread, what));
}

export async function removeFromContext(item: number): Promise<void> {
  await personCommands.run("Remove from Context", "oxplow.context.remove", { item });
}

export async function clearContext(threadId: string): Promise<void> {
  await personCommands.run("Clear Context", "oxplow.context.clear", { thread: threadId });
}
