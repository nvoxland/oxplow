import { type Dispatch, type RefObject, type SetStateAction, useEffect } from "react";

import {
  type AgentStatusEntry,
  getThreadState,
  listAgentStatuses,
  listStreams,
  onRemoteReconnect,
  type Stream,
  subscribeAgentStatus,
  subscribeOxplowEvents,
  subscribeWorkspaceContext,
  type ThreadState,
  type WorkspaceContext,
} from "./api.js";
import { showToast } from "./components/toastStore.js";
import { readsChanged, readsOf } from "./viz/rerun.js";

/** The thread state each stream shows is read from `v_thread`. */
const THREAD_READS = readsOf("v_thread");
/** The stream list is read from `v_stream`. */
const STREAM_READS = readsOf("v_stream");
import { logUi } from "./logger.js";
import { sessionStatusKey } from "./agentStatusRollup.js";

/**
 * Backend-event subscription wiring for the app shell, lifted out of
 * `App.tsx`. Each subscription lives in its own effect with an empty
 * dependency list — every input is either a stable `useState` dispatcher
 * passed in by the caller or an imported singleton, so none of these
 * re-subscribe across renders.
 */
export interface BackendSubscriptionHandlers {
  /** Which streams' thread state is loaded, to re-read when `v_thread` changes. */
  threadStatesRef: RefObject<Record<string, ThreadState>>;
  setWorkspaceContext: (next: WorkspaceContext) => void;
  setThreadStates: Dispatch<SetStateAction<Record<string, ThreadState>>>;
  setStreams: Dispatch<SetStateAction<Stream[]>>;
  setStream: Dispatch<SetStateAction<Stream | null>>;
  /** Each agent session's status, by `sessionStatusKey`. */
  setSessionStatuses: Dispatch<SetStateAction<Record<string, AgentStatusEntry>>>;
}

/**
 * The api surface this hook depends on, injectable so tests can supply
 * fakes without globally mocking `./api.js` (bun's `mock.module` leaks
 * across files in one test process). Defaults to the real imports, so the
 * production call site passes only `handlers`.
 */
export interface BackendSubscriptionApi {
  subscribeWorkspaceContext: typeof subscribeWorkspaceContext;
  subscribeOxplowEvents: typeof subscribeOxplowEvents;
  getThreadState: typeof getThreadState;
  listStreams: typeof listStreams;
  subscribeAgentStatus: typeof subscribeAgentStatus;
  listAgentStatuses: typeof listAgentStatuses;
  onRemoteReconnect: typeof onRemoteReconnect;
}

const defaultApi: BackendSubscriptionApi = {
  subscribeWorkspaceContext,
  subscribeOxplowEvents,
  getThreadState,
  listStreams,
  subscribeAgentStatus,
  listAgentStatuses,
  onRemoteReconnect,
};

export function useBackendSubscriptions(
  handlers: BackendSubscriptionHandlers,
  api: BackendSubscriptionApi = defaultApi,
): void {
  const {
    threadStatesRef,
    setWorkspaceContext,
    setThreadStates,
    setStreams,
    setStream,
    setSessionStatuses,
  } = handlers;
  const {
    subscribeWorkspaceContext,
    subscribeOxplowEvents,
    getThreadState,
    listStreams,
    subscribeAgentStatus,
    listAgentStatuses,
    onRemoteReconnect,
  } = api;

  useEffect(() => {
    return subscribeWorkspaceContext((next) => setWorkspaceContext(next));
  }, [setWorkspaceContext]);

  // A thread changed (created, renamed, marked, closed, reordered — the
  // `thread.*` commands): re-read each loaded stream's thread state when a
  // commit touched `v_thread`.
  useEffect(() => {
    const unsubscribe = subscribeOxplowEvents((event) => {
      if (!readsChanged(event as Record<string, unknown>, THREAD_READS)) return;
      for (const streamId of Object.keys(threadStatesRef.current ?? {})) {
        void getThreadState(streamId)
          .then((state) => {
            setThreadStates((latest) => ({ ...latest, [streamId]: state }));
          })
          .catch((error) => {
            logUi("warn", "failed to refresh thread state after a change", {
              streamId,
              error: String(error),
            });
          });
      }
    });
    return unsubscribe;
  }, [threadStatesRef, setThreadStates]);

  // Refresh the stream list when a commit touched `v_stream` (the
  // `stream.*` commands, the branch reconciler, an orphaned worktree's
  // archive). Swap the currently-selected stream for its fresh copy so
  // content changes (e.g. the custom prompt) reflect live; if it disappeared
  // from the list (e.g. it was just archived), fall back to the primary so the
  // rail doesn't render against a stale id.
  useEffect(() => {
    const unsubscribe = subscribeOxplowEvents((event) => {
      if (!readsChanged(event as Record<string, unknown>, STREAM_READS)) return;
      void listStreams()
        .then((updated) => {
          setStreams(updated);
          setStream((prev) => {
            if (!prev) return prev;
            const fresh = updated.find((s) => s.id === prev.id);
            if (fresh) return fresh;
            const primary = updated.find((s) => s.kind === "primary");
            return primary ?? updated[0] ?? null;
          });
        })
        .catch((error) => {
          logUi("warn", "failed to refresh streams after a change", { error: String(error) });
        });
    });
    return unsubscribe;
  }, [setStreams, setStream]);

  // Backing worktree was deleted out from under a stream — runtime has
  // already archived it, we just surface the toast so the user knows why
  // the rail row vanished.
  useEffect(() => {
    const unsubscribe = subscribeOxplowEvents((event) => {
      if (event.kind !== "streamOrphaned") return;
      const title = typeof event.title === "string" ? event.title : "Stream";
      showToast({
        message: `“${title}” was closed: its worktree directory was deleted.`,
      });
    });
    return unsubscribe;
  }, []);

  useEffect(() => {
    let cancelled = false;
    const seed = () =>
      listAgentStatuses()
        .then((entries) => {
          if (cancelled) return;
          const next: Record<string, AgentStatusEntry> = {};
          for (const entry of entries) next[sessionStatusKey(entry)] = entry;
          setSessionStatuses(next);
        })
        .catch((error) => {
          logUi("warn", "failed to seed agent statuses", { error: String(error) });
        });
    void seed();
    const unsubscribe = subscribeAgentStatus("all", (entry) => {
      setSessionStatuses((prev) => ({ ...prev, [sessionStatusKey(entry)]: entry }));
    });
    // A session opened or closed: re-read, so a closed one's status drops
    // out of its thread's dot.
    const unsubSessions = subscribeOxplowEvents((event) => {
      if (event.kind === "modelsChanged" && event.models.includes("v_agent_session")) void seed();
    });
    const unsubReconnect = onRemoteReconnect(() => void seed());
    return () => {
      cancelled = true;
      unsubscribe();
      unsubSessions();
      unsubReconnect();
    };
  }, [setSessionStatuses]);
}
