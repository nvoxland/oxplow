/**
 * Each stream's threads, loaded once when asked for and re-read when a
 * commit touches `v_thread` (a thread created, renamed, promoted, closed,
 * reordered — the `thread.*` commands), so a list built from them never
 * goes stale until remount (tsk790). The api is injected so a test can
 * drive it.
 */
import { useCallback, useEffect, useRef, useState } from "react";

import { listThreads, subscribeOxplowEvents, type Thread } from "../api.js";
import { readsChanged, readsOf } from "../lens/lensRerun.js";

const THREAD_READS = readsOf("v_thread");

export interface StreamThreadsApi {
  listThreads: (streamId: string) => Promise<Thread[]>;
  subscribeOxplowEvents: typeof subscribeOxplowEvents;
}

const DEFAULT_API: StreamThreadsApi = { listThreads, subscribeOxplowEvents };

export function useStreamThreads(
  api: StreamThreadsApi = DEFAULT_API,
  onError: (message: string) => void,
): {
  threadsByStream: Record<string, Thread[]>;
  requestThreads: (streamId: string) => void;
  /** The stream's threads: cached, or loaded now (and kept re-read). */
  threadsFor: (streamId: string) => Promise<Thread[]>;
} {
  const [threadsByStream, setThreadsByStream] = useState<Record<string, Thread[]>>({});
  const cache = useRef<Record<string, Thread[]>>({});
  cache.current = threadsByStream;
  const loaded = useRef(new Set<string>());
  const apiRef = useRef(api);
  apiRef.current = api;
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  const load = useCallback((streamId: string) => {
    void apiRef.current
      .listThreads(streamId)
      .then((ts) => setThreadsByStream((prev) => ({ ...prev, [streamId]: ts })))
      .catch((e: unknown) => onErrorRef.current(String(e)));
  }, []);

  const requestThreads = useCallback(
    (streamId: string) => {
      if (!streamId || loaded.current.has(streamId)) return;
      loaded.current.add(streamId);
      load(streamId);
    },
    [load],
  );

  const threadsFor = useCallback(async (streamId: string) => {
    const cached = cache.current[streamId];
    if (cached) return cached;
    loaded.current.add(streamId);
    const threads = await apiRef.current.listThreads(streamId);
    setThreadsByStream((prev) => ({ ...prev, [streamId]: threads }));
    return threads;
  }, []);

  useEffect(
    () =>
      apiRef.current.subscribeOxplowEvents((event) => {
        if (!readsChanged(event as Record<string, unknown>, THREAD_READS)) return;
        for (const streamId of loaded.current) load(streamId);
      }),
    [load],
  );

  return { threadsByStream, requestThreads, threadsFor };
}
