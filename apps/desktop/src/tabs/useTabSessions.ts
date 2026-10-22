import { useCallback, useEffect, useState } from "react";

import { logUi } from "../logger.js";
import { shellStorage } from "../shellStorage.js";
import { emptySession, parseSession, serializeSession, type Session } from "./session.js";

/** Every thread's center tabs (`tabs/session.ts`), in one key. */
export const SESSIONS_STORAGE_KEY = "oxplow.layout.v7.sessions";

/** The stored sessions; nothing for text that isn't them. */
export function readPersistedSessions(): Record<string, Session> {
  try {
    const raw = shellStorage.getItem(SESSIONS_STORAGE_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return {};
    const out: Record<string, Session> = {};
    for (const [thread, value] of Object.entries(parsed as Record<string, unknown>)) {
      const session = parseSession(JSON.stringify(value));
      if (session) out[thread] = session;
    }
    return out;
  } catch (err) {
    logUi("warn", "failed to read the stored tab sessions", { error: String(err) });
    return {};
  }
}

export function writePersistedSessions(sessions: Record<string, Session>): void {
  try {
    const out: Record<string, unknown> = {};
    for (const [thread, session] of Object.entries(sessions)) out[thread] = JSON.parse(serializeSession(session));
    shellStorage.setItem(SESSIONS_STORAGE_KEY, JSON.stringify(out));
  } catch (err) {
    logUi("warn", "failed to store the tab sessions", { error: String(err) });
  }
}

/**
 * The window's tab sessions, one per thread, restored on start and
 * stored on every change. `update` applies a model operation to one
 * thread's session; App draws everything tab-shaped from the result.
 */
export function useTabSessions() {
  const [sessions, setSessions] = useState<Record<string, Session>>(() => readPersistedSessions());
  useEffect(() => writePersistedSessions(sessions), [sessions]);
  const update = useCallback((thread: string, f: (s: Session) => Session) => {
    setSessions((prev) => {
      const now = prev[thread] ?? emptySession();
      const next = f(now);
      return next === now && prev[thread] ? prev : { ...prev, [thread]: next };
    });
  }, []);
  return { sessions, update };
}
