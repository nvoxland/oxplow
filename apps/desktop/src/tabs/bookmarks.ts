/**
 * The person's bookmarks: pages starred at one scope — the
 * thread, its stream or the project. Read from `v_bookmark`, written
 * through the `bookmark.set` / `bookmark.remove` commands. From one thread
 * a page is bookmarked once across what it sees, so setting another scope
 * moves it. `.context/pages-and-tabs.md` → "Bookmarks".
 */
import { useCallback, useEffect, useState } from "react";

import { querySql, runCommand } from "../api.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { streamRowId, threadRowId } from "../modelIds.js";
import type { Reads, SqlCell, SqlQueryResult } from "../tauri-bridge/generated/bindings.js";
import { refFromTabId } from "./pageRefs.js";
import type { TabRef } from "./tabState.js";

export type BookmarkScope = "thread" | "stream" | "project";

export interface Bookmark {
  ref: TabRef;
  label: string | null;
  scope: BookmarkScope;
}

/** Who's looking: the selected thread and its stream. */
export interface BookmarkViewer {
  threadId: string | null;
  streamId: string | null;
}

/** What a viewer sees, newest first — a page once, at the narrowest scope
 *  it's bookmarked at. */
export const BOOKMARKS_SQL = `
  SELECT ref, label, scope FROM (
    SELECT ref, label, scope, added_at, id,
           row_number() OVER (PARTITION BY ref ORDER BY
             CASE scope WHEN 'thread' THEN 0 WHEN 'stream' THEN 1 ELSE 2 END) AS n
    FROM v_bookmark
    WHERE (scope = 'thread' AND thread_id = ?1)
       OR (scope = 'stream' AND stream_id = ?2)
       OR scope = 'project'
  ) WHERE n = 1
  ORDER BY added_at DESC, id DESC`;

export function bookmarksParams(viewer: BookmarkViewer): SqlCell[] {
  return [
    viewer.threadId ? threadRowId(viewer.threadId) : null,
    viewer.streamId ? streamRowId(viewer.streamId) : null,
  ];
}

export function bookmarksFromResult(result: SqlQueryResult): Bookmark[] {
  const at = (row: SqlCell[], name: string) => row[result.columns.indexOf(name)] ?? null;
  const out: Bookmark[] = [];
  for (const row of result.rows) {
    const ref = refFromTabId(String(at(row, "ref")));
    if (!ref) continue;
    const label = at(row, "label");
    out.push({ ref, label: label == null ? null : String(label), scope: String(at(row, "scope")) as BookmarkScope });
  }
  return out;
}

export async function readBookmarks(viewer: BookmarkViewer): Promise<{ bookmarks: Bookmark[]; reads: Reads }> {
  const res = await querySql(BOOKMARKS_SQL, bookmarksParams(viewer), 1_000);
  return { bookmarks: bookmarksFromResult(res), reads: res.reads };
}

/** The viewer's bookmarks, re-read when `v_bookmark` changes. */
export function useBookmarks(threadId: string | null, streamId: string | null): Bookmark[] {
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    let cancelled = false;
    void readBookmarks({ threadId, streamId }).then(
      (r) => {
        if (cancelled) return;
        setBookmarks(r.bookmarks);
        setReads(r.reads);
      },
      (e: unknown) => recordOpError({ label: "Read bookmarks", message: e instanceof Error ? e.message : String(e) }),
    );
    return () => {
      cancelled = true;
    };
  }, [threadId, streamId]);
  useEffect(() => load(), [load]);
  useRerunOnChange(reads, () => void load());
  return bookmarks;
}

/** The viewer as the commands name it: the thread, else the stream. */
function viewerInput(viewer: BookmarkViewer): Record<string, string> {
  if (viewer.threadId) return { thread: viewer.threadId };
  if (viewer.streamId) return { stream: viewer.streamId };
  return {};
}

export function setBookmarkInput(viewer: BookmarkViewer, ref: TabRef, label: string | null, scope: BookmarkScope) {
  return { ref: ref.id, page_kind: ref.kind, ...(label ? { label } : {}), scope, ...viewerInput(viewer) };
}

/** Bookmark `ref` at `scope`, moving it if it's at another. */
export async function setBookmark(viewer: BookmarkViewer, ref: TabRef, label: string | null, scope: BookmarkScope): Promise<void> {
  await runCommand("bookmark.set", setBookmarkInput(viewer, ref, label, scope));
}

/** Take `ref`'s bookmark off; the outcome carries the audit id its undo
 *  needs (`undoCommand`). */
export function removeBookmark(viewer: BookmarkViewer, refId: string): ReturnType<typeof runCommand> {
  return runCommand("bookmark.remove", { ref: refId, ...viewerInput(viewer) });
}
