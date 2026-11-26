import { useCallback, useEffect, useRef, useState } from "react";

import { forgetTabSession, listTabSessions, onRemoteReconnect, saveTabSession } from "../api.js";
import { logUi } from "../logger.js";
import type { TabSave, TabSession } from "../tauri-bridge/generated/bindings.js";
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

/** The changes made before the stored tabs arrived, made over them in order.
 *  Pure; exported for tests. */
export function applyEarly(
  stored: Record<string, Session>,
  early: ReadonlyArray<readonly [string, (s: Session) => Session]>,
): Record<string, Session> {
  const out = { ...stored };
  for (const [thread, f] of early) out[thread] = f(out[thread] ?? emptySession());
  return out;
}

interface Writer {
  save(thread: string, json: string, revision: number | null): Promise<TabSave>;
  forget(thread: string): Promise<void>;
}

type Step = { kind: "save"; json: string } | { kind: "forget" };

/** Each thread's writes, one step at a time: saves collapse to the newest,
 *  a forget keeps its place, and each save gives the revision the last
 *  answer named, so a late save never overwrites a newer one. The live
 *  window owns its device's tabs, so a refused save (the row moved on: an
 *  answer lost on the way, a page load before this one) is sent again over
 *  the revision the daemon named. A step that fails stays, and `retry` sends
 *  it again. Exported for tests. */
export function createSessionSaver(write: Writer, revisions: Record<string, number>) {
  const known = new Map<string, number>(Object.entries(revisions));
  const queues = new Map<string, Step[]>();
  const running = new Set<string>();
  const drain = async (thread: string) => {
    running.add(thread);
    try {
      let refused = 0;
      for (let queue = queues.get(thread); queue && queue.length > 0; queue = queues.get(thread)) {
        const step = queue[0]!;
        try {
          if (step.kind === "forget") {
            await write.forget(thread);
            known.delete(thread);
          } else {
            const answer = await write.save(thread, step.json, known.get(thread) ?? null);
            if (answer.revision === null) known.delete(thread);
            else known.set(thread, answer.revision);
            if (!answer.saved && refused++ === 0) continue; // once more, over where the row is
            if (!answer.saved) logUi("warn", "a thread's tabs were refused twice; a later change saves again", { threadId: thread });
          }
          refused = 0;
          queue.shift();
        } catch (err) {
          // It stays queued; `retry` (on a reconnect) sends it again.
          logUi("warn", "failed to store a thread's tabs", { threadId: thread, error: String(err) });
          return;
        }
      }
    } finally {
      running.delete(thread);
    }
  };
  const push = (thread: string, step: Step) => {
    const queue = queues.get(thread) ?? [];
    // A save waiting behind the one in flight is replaced by a newer one.
    const last = queue.length > (running.has(thread) ? 1 : 0) ? queue[queue.length - 1] : undefined;
    if (step.kind === "save" && last?.kind === "save") queue[queue.length - 1] = step;
    else queue.push(step);
    queues.set(thread, queue);
    if (!running.has(thread)) void drain(thread);
  };
  return {
    save: (thread: string, json: string) => push(thread, { kind: "save", json }),
    /** The thread's tabs go, after what is already on its way; its next save
     *  starts a new row. */
    forget: (thread: string) => push(thread, { kind: "forget" }),
    /** Send again what failed (the daemon is back). */
    retry() {
      for (const [thread, queue] of queues) if (queue.length > 0 && !running.has(thread)) void drain(thread);
    },
  };
}

/**
 * The window's tab sessions, one per thread: this device's, read from the
 * daemon (`tab_session`) once at start (again on a reconnect until that
 * read answers), and each thread's saved as it changes (`createSessionSaver`).
 * `loaded` is false until they are read: until then no thread is known to be
 * new, and a change is kept and made over the stored tabs when they arrive.
 * `update` applies a model operation to one thread's session; the window
 * (`window/useWindowTabs.ts`) draws everything tab-shaped from the result.
 */
export function useTabSessions() {
  const [sessions, setSessions] = useState<Record<string, Session>>({});
  const [loaded, setLoaded] = useState(false);
  const saverRef = useRef<ReturnType<typeof createSessionSaver> | null>(null);
  // What each thread's stored text is, as last read or saved.
  const savedRef = useRef<Record<string, string>>({});
  const earlyRef = useRef<Array<readonly [string, (s: Session) => Session]>>([]);
  const loadingRef = useRef(false);

  useEffect(() => {
    const load = async () => {
      if (loadingRef.current || saverRef.current) return;
      loadingRef.current = true;
      try {
        const stored = sessionsFromStored(await listTabSessions());
        savedRef.current = Object.fromEntries(Object.entries(stored.sessions).map(([t, s]) => [t, serializeSession(s)]));
        saverRef.current = createSessionSaver({ save: saveTabSession, forget: forgetTabSession }, stored.revisions);
        setSessions(applyEarly(stored.sessions, earlyRef.current));
        earlyRef.current = [];
        setLoaded(true);
      } catch (err) {
        logUi("warn", "failed to read the stored tab sessions", { error: String(err) });
      } finally {
        loadingRef.current = false;
      }
    };
    void load();
    // Not read yet (the daemon was down at start): read once it is back.
    // Read already: send again what failed while it was down.
    return onRemoteReconnect(() => {
      if (saverRef.current) saverRef.current.retry();
      else void load();
    });
  }, []);

  // Each thread whose text changed is saved.
  useEffect(() => {
    const saver = saverRef.current;
    if (!loaded || !saver) return;
    for (const [thread, session] of Object.entries(sessions)) {
      const json = serializeSession(session);
      if (savedRef.current[thread] === json) continue;
      savedRef.current[thread] = json;
      saver.save(thread, json);
    }
  }, [sessions, loaded]);

  const update = useCallback((thread: string, f: (s: Session) => Session) => {
    if (!saverRef.current) earlyRef.current.push([thread, f]);
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
  }, []);
  return { sessions, loaded, update, forget };
}
