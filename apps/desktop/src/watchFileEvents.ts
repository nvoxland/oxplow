/// Follow a stream's file renames and deletes (`fileEvents.ts`): `onEvent`
/// runs for each one logged after watching began, from any caller (the
/// file tree, an agent's command, a page). The open buffers follow them
/// (`editor/buffers.ts`).
import { querySql, subscribeOxplowEvents } from "./api.js";
import { FILE_EVENTS_CURSOR_SQL, FILE_EVENTS_SQL, fileEventsFromRows, type FileEvent } from "./fileEvents.js";
import { streamRowId } from "./modelIds.js";

/** Watch `streamId`'s renames and deletes; returns the stop. */
export function watchFileEvents(streamId: string, onEvent: (event: FileEvent) => void): () => void {
  let live = true;
  let cursor: number | null = null;
  let reading = false;
  const read = async () => {
    if (reading) return;
    reading = true;
    try {
      if (cursor === null) {
        const first = await querySql(FILE_EVENTS_CURSOR_SQL, [], 1);
        cursor = Number(first.rows[0]?.[0] ?? 0);
        return;
      }
      const res = await querySql(FILE_EVENTS_SQL, [cursor, streamRowId(streamId)], 200);
      const next = fileEventsFromRows(res.rows, cursor);
      cursor = next.cursor;
      if (live) for (const e of next.events) onEvent(e);
    } catch {
      // Missed this time: the next commit to the log reads from the same cursor.
    } finally {
      reading = false;
    }
  };
  void read();
  const off = subscribeOxplowEvents((event) => {
    const datasets = (event as { datasets?: unknown }).datasets;
    if (event.kind === "datasetsChanged" && Array.isArray(datasets) && datasets.includes("v_event")) void read();
  });
  return () => {
    live = false;
    off();
  };
}
