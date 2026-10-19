/// Follow a stream's file renames and deletes (`fileEvents.ts`): the
/// handlers run for each one logged after the window started listening,
/// from any caller.
import { useEffect, useRef } from "react";

import { querySql, subscribeOxplowEvents } from "./api.js";
import { FILE_EVENTS_CURSOR_SQL, FILE_EVENTS_SQL, fileEventsFromRows, type FileEvent } from "./fileEvents.js";
import { streamRowId } from "./modelIds.js";

export function useFileEvents(streamId: string | null, onEvent: (stream: string, event: FileEvent) => void): void {
  const handler = useRef(onEvent);
  handler.current = onEvent;
  useEffect(() => {
    if (!streamId) return;
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
        if (live) for (const e of next.events) handler.current(streamId, e);
      } catch {
        // Missed this time: the next commit to the log reads from the same cursor.
      } finally {
        reading = false;
      }
    };
    void read();
    const off = subscribeOxplowEvents((event) => {
      const models = (event as { models?: unknown }).models;
      if (event.kind === "modelsChanged" && Array.isArray(models) && models.includes("v_event")) void read();
    });
    return () => {
      live = false;
      off();
    };
  }, [streamId]);
}
