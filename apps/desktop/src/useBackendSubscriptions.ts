import { type Dispatch, type RefObject, type SetStateAction, useEffect } from "react";

import {
  type AgentStatusEntry,
  generatedPaths,
  getConfig,
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
import { readWorkList, type WorkList } from "./workItems.js";
import { NO_READS, readsChanged, readsOf, unionReads } from "./lens/lensRerun.js";

/** The thread state each stream shows is read from `v_thread`. */
const THREAD_READS = readsOf("v_thread");
/** The stream list is read from `v_stream`. */
const STREAM_READS = readsOf("v_stream");
import type { Reads } from "./tauri-bridge/generated/bindings.js";
import { logUi } from "./logger.js";
import { sessionStatusKey } from "./agentStatusRollup.js";

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
  threadWorkStatesRef: RefObject<Record<string, WorkList>>;
  /** Which streams' thread state is loaded, to re-read when `v_thread` changes. */
  threadStatesRef: RefObject<Record<string, ThreadState>>;
  setWorkspaceContext: (next: WorkspaceContext) => void;
  setBacklogState: (next: WorkList) => void;
  setThreadWorkStates: Dispatch<SetStateAction<Record<string, WorkList>>>;
  setThreadStates: Dispatch<SetStateAction<Record<string, ThreadState>>>;
  setStreams: Dispatch<SetStateAction<Stream[]>>;
  setStream: Dispatch<SetStateAction<Stream | null>>;
  /** Each agent session's status, by `sessionStatusKey`. */
  setSessionStatuses: Dispatch<SetStateAction<Record<string, AgentStatusEntry>>>;
  setGeneratedState: (next: { exclude: string[]; include: string[] }) => void;
}

/**
 * The api surface this hook depends on, injectable so tests can supply
 * fakes without globally mocking `./api.js` (bun's `mock.module` leaks
 * across files in one test process). Defaults to the real imports, so the
 * production call site passes only `handlers`.
 */
export interface BackendSubscriptionApi {
  subscribeWorkspaceContext: typeof subscribeWorkspaceContext;
  readWorkList: typeof readWorkList;
  subscribeOxplowEvents: typeof subscribeOxplowEvents;
  getThreadState: typeof getThreadState;
  listStreams: typeof listStreams;
  subscribeAgentStatus: typeof subscribeAgentStatus;
  listAgentStatuses: typeof listAgentStatuses;
  getConfig: typeof getConfig;
  onRemoteReconnect: typeof onRemoteReconnect;
}

const defaultApi: BackendSubscriptionApi = {
  subscribeWorkspaceContext,
  readWorkList,
  subscribeOxplowEvents,
  getThreadState,
  listStreams,
  subscribeAgentStatus,
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
    threadStatesRef,
    setWorkspaceContext,
    setBacklogState,
    setThreadWorkStates,
    setThreadStates,
    setStreams,
    setStream,
    setSessionStatuses,
    setGeneratedState,
  } = handlers;
  const {
    subscribeWorkspaceContext,
    readWorkList,
    subscribeOxplowEvents,
    getThreadState,
    listStreams,
    subscribeAgentStatus,
    listAgentStatuses,
    getConfig,
    onRemoteReconnect,
  } = api;

  useEffect(() => {
    return subscribeWorkspaceContext((next) => setWorkspaceContext(next));
  }, [setWorkspaceContext]);

  // Work lists are read through the work-item interface: the backlog and every loaded
  // thread's work re-read when a model they read changes — whoever wrote
  // it (a person's command, an agent's MCP tool). What they read is the
  // union of the reads the last loads reported (`readsChanged`, the same
  // rule as `useRerunOnChange`), not a list of task models.
  useEffect(() => {
    let cancelled = false;
    const reads: { backlog: Reads; threads: Record<string, Reads> } = { backlog: NO_READS, threads: {} };
    const allReads = () => unionReads([reads.backlog, ...Object.values(reads.threads)]);
    const reloadThread = (threadId: string) =>
      readWorkList(threadId)
        .then((work) => {
          if (cancelled) return;
          reads.threads[threadId] = work.reads;
          setThreadWorkStates((prev) => ({ ...prev, [threadId]: work }));
        })
        .catch((error) => logUi("warn", "failed to refresh thread work", { threadId, error: String(error) }));
    const reloadAll = () => {
      void readWorkList(null)
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

  // A thread changed (created, renamed, promoted, closed, reordered — the
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
    const reload = () => {
      void getConfig()
        .then((cfg) => {
          if (cancelled) return;
          setGeneratedState(generatedPaths(cfg));
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
  }, [setGeneratedState]);

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
