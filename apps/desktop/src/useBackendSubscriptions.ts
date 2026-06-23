import { type Dispatch, type RefObject, type SetStateAction, useEffect } from "react";

import {
  type AgentKind,
  type AgentStatus,
  type BacklogState,
  formatAgentStallAlert,
  getBacklogState,
  getConfig,
  getThreadState,
  getThreadWorkState,
  listAgentStatuses,
  listStreams,
  onRemoteReconnect,
  type Stream,
  subscribeAgentStallAlerts,
  subscribeAgentStatus,
  subscribeBacklogEvents,
  subscribeOxplowEvents,
  subscribeTaskEvents,
  subscribeWorkspaceContext,
  type ThreadState,
  type ThreadWorkState,
  type WorkspaceContext,
} from "./api.js";
import { showToast } from "./components/toastStore.js";
import { logUi } from "./logger.js";

/**
 * Backend-event subscription wiring for the app shell, lifted out of
 * `App.tsx`. Each subscription lives in its own effect with an empty
 * dependency list — every input is either a stable `useState` dispatcher
 * passed in by the caller or an imported singleton, so none of these
 * re-subscribe across renders.
 *
 * `threadStatesRef` is the one piece of mutable state a callback needs to
 * read (the followup.changed handler recovers a stream id from the cached
 * thread map). It's a ref rather than a value so the subscription can stay
 * mounted once instead of tearing down and re-subscribing on every thread
 * change — the source of a long-standing re-subscribe churn in App.tsx.
 */
export interface BackendSubscriptionHandlers {
  threadStatesRef: RefObject<Record<string, ThreadState>>;
  setWorkspaceContext: (next: WorkspaceContext) => void;
  setBacklogState: (next: BacklogState) => void;
  setThreadWorkStates: Dispatch<SetStateAction<Record<string, ThreadWorkState>>>;
  setThreadStates: Dispatch<SetStateAction<Record<string, ThreadState>>>;
  setStreams: Dispatch<SetStateAction<Stream[]>>;
  setStream: Dispatch<SetStateAction<Stream | null>>;
  setAgentStatuses: Dispatch<SetStateAction<Record<string, AgentStatus>>>;
  setGeneratedState: (next: { exclude: string[]; include: string[] }) => void;
  setEnabledAgents: (next: AgentKind[]) => void;
}

/**
 * The api surface this hook depends on, injectable so tests can supply
 * fakes without globally mocking `./api.js` (bun's `mock.module` leaks
 * across files in one test process). Defaults to the real imports, so the
 * production call site passes only `handlers`.
 */
export interface BackendSubscriptionApi {
  subscribeWorkspaceContext: typeof subscribeWorkspaceContext;
  getBacklogState: typeof getBacklogState;
  subscribeBacklogEvents: typeof subscribeBacklogEvents;
  subscribeTaskEvents: typeof subscribeTaskEvents;
  getThreadWorkState: typeof getThreadWorkState;
  subscribeOxplowEvents: typeof subscribeOxplowEvents;
  getThreadState: typeof getThreadState;
  listStreams: typeof listStreams;
  subscribeAgentStatus: typeof subscribeAgentStatus;
  subscribeAgentStallAlerts: typeof subscribeAgentStallAlerts;
  listAgentStatuses: typeof listAgentStatuses;
  getConfig: typeof getConfig;
  onRemoteReconnect: typeof onRemoteReconnect;
}

const defaultApi: BackendSubscriptionApi = {
  subscribeWorkspaceContext,
  getBacklogState,
  subscribeBacklogEvents,
  subscribeTaskEvents,
  getThreadWorkState,
  subscribeOxplowEvents,
  getThreadState,
  listStreams,
  subscribeAgentStatus,
  subscribeAgentStallAlerts,
  listAgentStatuses,
  getConfig,
  onRemoteReconnect,
};

export function useBackendSubscriptions(
  handlers: BackendSubscriptionHandlers,
  api: BackendSubscriptionApi = defaultApi,
): void {
  const {
    threadStatesRef,
    setWorkspaceContext,
    setBacklogState,
    setThreadWorkStates,
    setThreadStates,
    setStreams,
    setStream,
    setAgentStatuses,
    setGeneratedState,
    setEnabledAgents,
  } = handlers;
  const {
    subscribeWorkspaceContext,
    getBacklogState,
    subscribeBacklogEvents,
    subscribeTaskEvents,
    getThreadWorkState,
    subscribeOxplowEvents,
    getThreadState,
    listStreams,
    subscribeAgentStatus,
    subscribeAgentStallAlerts,
    listAgentStatuses,
    getConfig,
    onRemoteReconnect,
  } = api;

  useEffect(() => {
    return subscribeWorkspaceContext((next) => setWorkspaceContext(next));
  }, [setWorkspaceContext]);

  useEffect(() => {
    let cancelled = false;
    const loadBacklog = () =>
      getBacklogState()
        .then((state) => {
          if (!cancelled) setBacklogState(state);
        })
        .catch((error) => logUi("warn", "failed to refresh backlog state", { error: String(error) }));
    void loadBacklog();
    const unsubscribe = subscribeBacklogEvents(() => void loadBacklog());
    // Re-hydrate after a remote-daemon WS reconnect (events missed
    // while the socket was down).
    const unsubReconnect = onRemoteReconnect(() => void loadBacklog());
    return () => {
      cancelled = true;
      unsubscribe();
      unsubReconnect();
    };
  }, [setBacklogState]);

  useEffect(() => {
    const unsubscribe = subscribeTaskEvents("all", (event) => {
      void getThreadWorkState(event.streamId, event.threadId)
        .then((workState) => {
          setThreadWorkStates((prev) => ({ ...prev, [event.threadId]: workState }));
        })
        .catch((error) => {
          logUi("warn", "failed to refresh thread work state after change event", {
            streamId: event.streamId,
            threadId: event.threadId,
            kind: event.kind,
            error: String(error),
          });
        });
    });
    return unsubscribe;
  }, [setThreadWorkStates]);

  // Followups are transient (in-memory), but we still want the Ready
  // section to live-update when the agent adds/removes one mid-turn.
  // Re-fetch the same ThreadWorkState envelope (followups are layered
  // in by the tasks API wrapper) after every followup.changed event.
  // Stream id is recovered from the cached threadState map — the event
  // itself only carries threadId. Read the ref (not a closed-over value)
  // so this subscription stays mounted instead of re-subscribing on
  // every thread change.
  useEffect(() => {
    const unsubscribe = subscribeOxplowEvents((event) => {
      if (event.kind !== "followupsChanged") return;
      const threadId = event.threadId;
      let streamIdForThread: string | null = null;
      for (const [sid, state] of Object.entries(threadStatesRef.current ?? {})) {
        if (state.threads.some((t) => t.id === threadId)) {
          streamIdForThread = sid;
          break;
        }
      }
      if (!streamIdForThread) return;
      void getThreadWorkState(streamIdForThread, threadId)
        .then((workState) => {
          setThreadWorkStates((prev) => ({ ...prev, [threadId]: workState }));
        })
        .catch((error) => {
          logUi("warn", "failed to refresh thread work state after followup.changed", {
            threadId,
            error: String(error),
          });
        });
    });
    return unsubscribe;
  }, [threadStatesRef, setThreadWorkStates]);

  useEffect(() => {
    const unsubscribe = subscribeOxplowEvents((event) => {
      if (event.kind !== "threadsChanged") return;
      void getThreadState(event.streamId)
        .then((state) => {
          setThreadStates((prev) => ({ ...prev, [event.streamId]: state }));
        })
        .catch((error) => {
          logUi("warn", "failed to refresh thread state after change event", {
            streamId: event.streamId,
            kind: event.kind,
            error: String(error),
          });
        });
    });
    return unsubscribe;
  }, [setThreadStates]);

  // Refresh the stream list whenever the cross-store bus signals a
  // `streamsChanged` (creation, archive via Remove…, rename, reorder,
  // prompt edit). Swap the currently-selected stream for its fresh copy so
  // content changes (e.g. the custom prompt) reflect live; if it disappeared
  // from the list (e.g. it was just archived), fall back to the primary so the
  // rail doesn't render against a stale id.
  useEffect(() => {
    const unsubscribe = subscribeOxplowEvents((event) => {
      if (event.kind !== "streamsChanged") return;
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
          logUi("warn", "failed to refresh streams after streamsChanged", { error: String(error) });
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
    const reload = () => {
      void getConfig()
        .then((cfg) => {
          if (cancelled) return;
          setGeneratedState(cfg.generated);
          setEnabledAgents(cfg.agents?.length ? cfg.agents : ["claude"]);
        })
        .catch((error) => {
          logUi("warn", "failed to load config", { error: String(error) });
        });
    };
    reload();
    const unsub = subscribeOxplowEvents((event) => {
      if (event.kind === "configChanged") reload();
    });
    const unsubReconnect = onRemoteReconnect(() => reload());
    return () => {
      cancelled = true;
      unsub();
      unsubReconnect();
    };
  }, [setEnabledAgents, setGeneratedState]);

  useEffect(() => {
    let cancelled = false;
    const seed = () =>
      listAgentStatuses()
        .then((entries) => {
          if (cancelled) return;
          const next: Record<string, AgentStatus> = {};
          for (const entry of entries) next[entry.threadId] = entry.status;
          setAgentStatuses(next);
        })
        .catch((error) => {
          logUi("warn", "failed to seed agent statuses", { error: String(error) });
        });
    void seed();
    const unsubscribe = subscribeAgentStatus("all", (entry) => {
      setAgentStatuses((prev) => ({ ...prev, [entry.threadId]: entry.status }));
    });
    const unsubReconnect = onRemoteReconnect(() => void seed());
    return () => {
      cancelled = true;
      unsubscribe();
      unsubReconnect();
    };
  }, [setAgentStatuses]);

  useEffect(() => {
    // Stall watchdog nudge: the backend fires this once per stall
    // episode when in_progress work sits on a non-running agent past
    // the alert threshold. Surface it as a toast (no onUndo — it is
    // informational; the fix is to re-prompt the agent).
    return subscribeAgentStallAlerts((alert) => {
      showToast({ message: formatAgentStallAlert(alert), actionLabel: "Dismiss" });
    });
  }, []);
}
