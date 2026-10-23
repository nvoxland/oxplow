/// What an agent shows the person (`oxplow_show_lens`, `.context/extensions.md`
/// "Thread answers") opens as a tab of its thread: `watchShownAnswers`
/// calls `onShown` with each answer's URI recorded after watching began,
/// or in the ten seconds before (the daemon's clock), so one shown while
/// the window was still starting isn't missed.
import { querySql, subscribeOxplowEvents } from "./api.js";
import { threadRowId } from "./modelIds.js";

const CURSOR_SQL = `SELECT coalesce(max(id), 0) FROM v_thread_answer
  WHERE thread_id = ?1 AND created_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-10 seconds')`;
const NEW_SQL = "SELECT id, uri FROM v_thread_answer WHERE thread_id = ?1 AND id > ?2 ORDER BY id";

/** Watch `threadId`'s answers; returns the stop. */
export function watchShownAnswers(threadId: string, onShown: (uri: string) => void): () => void {
  let live = true;
  let cursor: number | null = null;
  let reading = false;
  const thread = threadRowId(threadId);
  const read = async () => {
    if (reading) return;
    reading = true;
    try {
      if (cursor === null) {
        const first = await querySql(CURSOR_SQL, [thread], 1);
        cursor = Number(first.rows[0]?.[0] ?? 0);
      }
      const res = await querySql(NEW_SQL, [thread, cursor], 50);
      for (const [id, uri] of res.rows) {
        cursor = Math.max(cursor, Number(id));
        if (live && typeof uri === "string") onShown(uri);
      }
    } catch {
      // Missed this time: the next change reads from the same cursor.
    } finally {
      reading = false;
    }
  };
  void read();
  const off = subscribeOxplowEvents((event) => {
    const models = (event as { models?: unknown }).models;
    if (event.kind === "modelsChanged" && Array.isArray(models) && models.includes("v_thread_answer")) void read();
  });
  return () => {
    live = false;
    off();
  };
}
