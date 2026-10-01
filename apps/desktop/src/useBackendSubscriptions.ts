import { type Dispatch, type RefObject, type SetStateAction, useEffect } from "react";

import {
  type AgentKind,
  type AgentStatus,
  type BacklogState,
  formatAgentStallAlert,
  getConfig,
  getThreadState,
  listAgentStatuses,
  listStreams,
  onRemoteReconnect,
  type Stream,
  subscribeAgentStallAlerts,
  subscribeAgentStatus,
  subscribeOxplowEvents,
  subscribeWorkspaceContext,
  type ThreadState,
  type ThreadWorkState,
  type WorkspaceContext,
} from "./api.js";
import { showToast } from "./components/toastStore.js";
import { readBacklog, readThreadWork } from "./workItems.js";
import { NO_READS, readsChanged, unionReads } from "./lens/lensRerun.js";
import type { Reads } from "./tauri-bridge/generated/bindings.js";
import { logUi } from "./logger.js";

/**
 * Backend-event subscription wiring for the app shell, lifted out of
 * `App.tsx`. Each subscription lives in its own effect with an empty
 * dependency list — every input is either a stable `useState` dispatcher
 * passed in by the caller or an imported singleton, so none of these
 * re-subscribe across renders.
 *
 * `threadWorkStatesRef` is the one piece of mutable state a callback needs
 * to read (which threads' work is loaded, to re-read when a task model
 * changes). It's a ref rather than a value so the subscription can stay
 * mounted once instead of tearing down and re-subscribing on every change.
 */
export interface BackendSubscriptionHandlers {
  threadWorkStatesRef: RefObject<Record<string, ThreadWorkState>>;
  setWorkspaceContext: (next: WorkspaceContext) => void;
  setBacklogState: (next: BacklogState) => void;
  setThreadWorkStates: Dispatch<SetStateAction<Record<string, ThreadWorkState>>>;
  setThreadStates: Dispatch<SetStateAction<Record<string, ThreadState>>>;
  setStreams: Dispatch<SetStateAction<Stream[]>>;
  setStream: Dispatch<SetStateAction<Stream | null>>;
  setAgentStatuses: Dispatch<SetStateAction<Record<string, AgentStatus>>>;
  setAgentQuestions: Dispatch<SetStateAction<Record<string, string | undefined>>>;
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
  readBacklog: typeof readBacklog;
  readThreadWork: typeof readThreadWork;
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
  readBacklog,
  readThreadWork,
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
    threadWorkStatesRef,
    setWorkspaceContext,
    setBacklogState,
    setThreadWorkStates,
    setThreadStates,
    setStreams,
    setStream,
    setAgentStatuses,
    setAgentQuestions,
    setGeneratedState,
    setEnabledAgents,
  } = handlers;
  const {
    subscribeWorkspaceContext,
    readBacklog,
    readThreadWork,
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

  // Tasks are read from the models (P6.E1b): the backlog and every loaded
  // thread's work re-read when a model they read changes — whoever wrote
  // it (a person's command, an agent's MCP tool). What they read is the
  // union of the reads the last loads reported (`readsChanged`, the same
  // rule as `useRerunOnChange`), not a list of task models.
  useEffect(() => {
    let cancelled = false;
    const reads: { backlog: Reads; threads: Record<string, Reads> } = { backlog: NO_READS, threads: {} };
    const allReads = () => unionReads([reads.backlog, ...Object.values(reads.threads)]);
    const reloadThread = (threadId: string) =>
      readThreadWork(threadId)
        .then((work) => {
          if (cancelled) return;
          reads.threads[threadId] = work.reads;
          setThreadWorkStates((prev) => ({ ...prev, [threadId]: work }));
        })
        .catch((error) => logUi("warn", "failed to refresh thread work", { threadId, error: String(error) }));
    const reloadAll = () => {
      void readBacklog()
        .then((state) => {
          if (cancelled) return;
          reads.backlog = state.reads;
          setBacklogState(state);
        })
        .catch((error) => logUi("warn", "failed to refresh backlog state", { error: String(error) }));
      for (const threadId of Object.keys(threadWorkStatesRef.current ?? {})) void reloadThread(threadId);
    };
    reloadAll();
    const unsubscribe = subscribeOxplowEvents((event) => {
      if (readsChanged(event as Record<string, unknown>, allReads())) reloadAll();
      // Followups are in-memory, not a model: their own event.
      else if (event.kind === "followupsChanged") void reloadThread(event.threadId as string);
    });
    // Re-hydrate after a remote-daemon WS reconnect (events missed while
    // the socket was down).
    const unsubReconnect = onRemoteReconnect(reloadAll);
    return () => {
      cancelled = true;
      unsubscribe();
      unsubReconnect();
    };
  }, [threadWorkStatesRef, setBacklogState, setThreadWorkStates]);

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
          const nextQ: Record<string, string | undefined> = {};
          for (const entry of entries) {
            next[entry.threadId] = entry.status;
            nextQ[entry.threadId] = entry.question;
          }
          setAgentStatuses(next);
          setAgentQuestions(nextQ);
        })
        .catch((error) => {
          logUi("warn", "failed to seed agent statuses", { error: String(error) });
        });
    void seed();
    const unsubscribe = subscribeAgentStatus("all", (entry) => {
      setAgentStatuses((prev) => ({ ...prev, [entry.threadId]: entry.status }));
      setAgentQuestions((prev) => ({ ...prev, [entry.threadId]: entry.question }));
    });
    const unsubReconnect = onRemoteReconnect(() => void seed());
    return () => {
      cancelled = true;
      unsubscribe();
      unsubReconnect();
    };
  }, [setAgentStatuses, setAgentQuestions]);

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
