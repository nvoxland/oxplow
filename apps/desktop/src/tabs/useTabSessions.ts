import { useCallback, useEffect, useRef, useState } from "react";

import { forgetTabSession, listTabSessions, onRemoteReconnect, saveTabSession } from "../api.js";
import { logUi } from "../logger.js";
import type { TabSession } from "../tauri-bridge/generated/bindings.js";
import { emptySession, parseSession, serializeSession, type Session } from "./session.js";

/** The stored tabs by thread, and the revision each was read at; a thread
 *  whose text isn't a session drops alone. Pure; exported for tests. */
export function sessionsFromStored(stored: TabSession[]): {
  sessions: Record<string, Session>;
  revisions: Record<string, number>;
} {
  const sessions: Record<string, Session> = {};
  const revisions: Record<string, number> = {};
  for (const row of stored) {
    const session = parseSession(row.sessionJson);
    if (!session) {
      logUi("warn", "a stored tab session isn't one; the thread opens as new", { threadId: row.threadId });
      continue;
    }
    sessions[row.threadId] = session;
    revisions[row.threadId] = row.revision;
  }
  return { sessions, revisions };
}

type Write = (thread: string, json: string, revision: number | null) => Promise<number | null>;

/** Each thread's saves, one at a time: while one is in flight only the
 *  newest waits, and each gives the revision the last one answered, so a
 *  late save never overwrites a newer one. A save the daemon refuses as
 *  stale (its tabs moved on elsewhere) is told to `stale`. Exported for
 *  tests. */
export function createSessionSaver(write: Write, revisions: Record<string, number>, stale: (thread: string) => void) {
  const known = new Map<string, number>(Object.entries(revisions));
  const pending = new Map<string, string>();
  const running = new Set<string>();
  const drain = async (thread: string) => {
    running.add(thread);
    try {
      for (let json = pending.get(thread); json !== undefined; json = pending.get(thread)) {
        pending.delete(thread);
        try {
          const next = await write(thread, json, known.get(thread) ?? null);
          if (next === null) {
            stale(thread);
            return;
          }
          known.set(thread, next);
        } catch (err) {
          // A later change saves again.
          logUi("warn", "failed to store a thread's tabs", { threadId: thread, error: String(err) });
          return;
        }
      }
    } finally {
      running.delete(thread);
    }
  };
  return {
    save(thread: string, json: string) {
      pending.set(thread, json);
      if (!running.has(thread)) void drain(thread);
    },
    /** The thread's tabs are gone: its next save starts a new row. */
    forget(thread: string) {
      pending.delete(thread);
      known.delete(thread);
    },
    /** What the daemon has now (after a stale save or a reload). */
    adopt(thread: string, revision: number) {
      known.set(thread, revision);
    },
  };
}

/**
 * The window's tab sessions, one per thread: this device's, read from the
 * daemon (`tab_session`) at start and on a reconnect until they load, and
 * each thread's saved as it changes. `loaded` is false until they are read:
 * until then no thread is known to be new. `update` applies a model
 * operation to one thread's session; the window (`window/useWindowTabs.ts`)
 * draws everything tab-shaped from the result.
 */
export function useTabSessions() {
  const [sessions, setSessions] = useState<Record<string, Session>>({});
  const [loaded, setLoaded] = useState(false);
  const saverRef = useRef<ReturnType<typeof createSessionSaver> | null>(null);
  const savedRef = useRef<Record<string, Session>>({});

  const load = useCallback(async (only?: string) => {
    const stored = sessionsFromStored(await listTabSessions());
    if (only !== undefined) {
      // A stale save: this thread takes the daemon's copy.
      const session = stored.sessions[only];
      setSessions((prev) => {
        if (session) return { ...prev, [only]: session };
        const { [only]: _, ...rest } = prev;
        return rest;
      });
      if (session) savedRef.current[only] = session;
      saverRef.current?.adopt(only, stored.revisions[only] ?? 0);
      return;
    }
    savedRef.current = { ...stored.sessions };
    saverRef.current = createSessionSaver(saveTabSession, stored.revisions, (thread) => {
      void load(thread).catch((err) => logUi("warn", "failed to read a thread's stored tabs", { error: String(err) }));
    });
    // A change made before they arrived stays (and is saved).
    setSessions((prev) => ({ ...stored.sessions, ...prev }));
    setLoaded(true);
  }, []);

  useEffect(() => {
    const first = () =>
      load().catch((err) => logUi("warn", "failed to read the stored tab sessions", { error: String(err) }));
    void first();
    // Not read yet (the daemon was down at start): read once it is back.
    return onRemoteReconnect(() => {
      if (!saverRef.current) void first();
    });
  }, [load]);

  // Each thread whose session changed is saved.
  useEffect(() => {
    const saver = saverRef.current;
    if (!loaded || !saver) return;
    for (const [thread, session] of Object.entries(sessions)) {
      if (savedRef.current[thread] === session) continue;
      savedRef.current[thread] = session;
      saver.save(thread, serializeSession(session));
    }
  }, [sessions, loaded]);

  const update = useCallback((thread: string, f: (s: Session) => Session) => {
    setSessions((prev) => {
      const now = prev[thread] ?? emptySession();
      const next = f(now);
      return next === now && prev[thread] ? prev : { ...prev, [thread]: next };
    });
  }, []);
  /** Forget a thread's stored tabs: it opens as a new thread. */
  const forget = useCallback((thread: string) => {
    setSessions((prev) => {
      if (!(thread in prev)) return prev;
      const { [thread]: _, ...rest } = prev;
      return rest;
    });
    delete savedRef.current[thread];
    saverRef.current?.forget(thread);
    void forgetTabSession(thread).catch((err) =>
      logUi("warn", "failed to forget a thread's stored tabs", { threadId: thread, error: String(err) }),
    );
  }, []);
  return { sessions, loaded, update, forget };
}
