// Persistence for the open files of each stream (their
// paths; content and drafts are read again). The tabs themselves are
// `useTabSessions`'s. Readers are total: a parse failure is "nothing
// persisted", never a throw.

import type { FileSessionState } from "../editor-session.js";
import { logUi } from "../logger.js";
import { shellStorage } from "../shellStorage.js";

export const FILE_SESSIONS_STORAGE_KEY = "oxplow.layout.v2.fileSessions";

export function readPersistedFileSessionPaths(): Record<string, string[]> {
  try {
    const raw = shellStorage.getItem(FILE_SESSIONS_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return {};
    const out: Record<string, string[]> = {};
    for (const [streamId, paths] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof streamId !== "string") continue;
      if (!Array.isArray(paths)) continue;
      const clean = paths.filter((p): p is string => typeof p === "string");
      if (clean.length > 0) out[streamId] = clean;
    }
    return out;
  } catch (err) {
    logUi("warn", "failed to parse persisted file sessions", { error: String(err) });
    return {};
  }
}

export function writePersistedFileSessionPaths(sessions: Record<string, FileSessionState>): void {
  try {
    const out: Record<string, string[]> = {};
    for (const [streamId, session] of Object.entries(sessions)) {
      if (session.openOrder.length > 0) out[streamId] = session.openOrder;
    }
    shellStorage.setItem(FILE_SESSIONS_STORAGE_KEY, JSON.stringify(out));
  } catch {}
}
