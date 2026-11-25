import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  closeThread,
  createThread,
  getCurrentStream,
  getThreadState,
  getWorkspaceContext,
  listStreams,
  markThread,
  onRemoteReconnect,
  renameStream,
  renameThread,
  selectThread,
  switchStream,
  type Stream,
  type ThreadMark,
  type ThreadState,
  type WorkspaceContext,
} from "../api.js";
import { runForPerson } from "../commands/runForPerson.js";
import { logUi } from "../logger.js";

const NO_THREADS: ThreadState = { selectedThreadId: null, threads: [] };

export interface WindowStreamsOptions {
  /** The window's error banner: a load or switch that fails says why, and a
   *  good one clears it. */
  setError: (error: string | null) => void;
  /** Whether the daemon answered the last load. */
  setDaemonUnavailable: (unavailable: boolean) => void;
}

/** The window's streams and each stream's threads, the shown stream and its
 *  selected thread, and the person's stream and thread commands. Loaded on
 *  mount and again on a reconnect (`onRemoteReconnect`), so the window
 *  catches up on what it missed without a reload. Shell-neutral: any shell
 *  draws its own switcher over it. */
export function useWindowStreams({ setError, setDaemonUnavailable }: WindowStreamsOptions) {
  const [streams, setStreams] = useState<Stream[]>([]);
  const [stream, setStream] = useState<Stream | null>(null);
  const [threadStates, setThreadStates] = useState<Record<string, ThreadState>>({});
  const [workspaceContext, setWorkspaceContext] = useState<WorkspaceContext>({ vcsEnabled: false });
  // Mirror of threadStates for subscription callbacks that need the
  // latest map without re-subscribing when it changes (see
  // useBackendSubscriptions). Kept current on every render.
  const threadStatesRef = useRef(threadStates);
  threadStatesRef.current = threadStates;
  const options = useRef({ setError, setDaemonUnavailable });
  options.current = { setError, setDaemonUnavailable };

  // Top-level state hydration: streams, the current stream + its
  // threads, workspace context, and the selected thread's work state.
  const loadInitialAppState = useCallback(() => {
    return Promise.all([listStreams(), getCurrentStream(), getWorkspaceContext()])
      .then(async ([allStreams, current, context]) => {
        const initialThreadState = await getThreadState(current.id);
        setStreams(allStreams);
        setStream(current);
        setThreadStates((prev) => ({ ...prev, [current.id]: initialThreadState }));
        setWorkspaceContext(context);
        // Prefetch thread state for the remaining streams so the
        // navigator can render every thread under every stream
        // immediately, not only after the user switches to that stream.
        const otherStreams = allStreams.filter((s) => s.id !== current.id);
        for (const s of otherStreams) {
          void getThreadState(s.id)
            .then((state) => setThreadStates((prev) => ({ ...prev, [s.id]: state })))
            .catch((e) => logUi("warn", "failed to prefetch thread state", { streamId: s.id, error: String(e) }));
        }
        options.current.setError(null);
        options.current.setDaemonUnavailable(false);
        logUi("info", "loaded initial app state", {
          streamCount: allStreams.length,
          currentStreamId: current.id,
          vcsEnabled: context.vcsEnabled,
        });
      })
      .catch((e) => {
        options.current.setError(String(e));
        options.current.setDaemonUnavailable(true);
        logUi("error", "failed to load initial app state", { error: String(e) });
      });
  }, []);

  useEffect(() => {
    void loadInitialAppState();
  }, [loadInitialAppState]);

  // Remote-daemon WS reconnect: re-hydrate so the window catches up on
  // events missed while the socket was down. The WS itself auto-re-
  // subscribes; this covers the snapshot the window holds.
  useEffect(() => {
    return onRemoteReconnect(() => {
      logUi("info", "remote daemon reconnected, resyncing state");
      void loadInitialAppState();
    });
  }, [loadInitialAppState]);

  async function handleSwitch(id: string) {
    try {
      logUi("info", "switching stream", { streamId: id });
      const next = await switchStream(id);
      let nextThreadState = threadStates[next.id] ?? await getThreadState(next.id);
      // Invariant: a stream must always have a selected thread. If the
      // switched-to stream has no remembered selection, auto-pick its
      // first thread by sort order and persist that choice so subsequent
      // switches remember it.
      if (!nextThreadState.selectedThreadId && nextThreadState.threads.length > 0) {
        nextThreadState = await selectThread(next.id, nextThreadState.threads[0].id);
      }
      setThreadStates((prev) => ({ ...prev, [next.id]: nextThreadState }));
      setStream(next);
      setError(null);
      setDaemonUnavailable(false);
      logUi("info", "switched stream", { streamId: next.id, title: next.title });
    } catch (e) {
      setError(String(e));
      logUi("error", "failed to switch stream", { streamId: id, error: String(e) });
    }
  }

  async function handleRenameStreamById(streamId: string, newTitle: string) {
    const updated = await renameStream(streamId, newTitle);
    if (stream?.id === updated.id) setStream(updated);
    setStreams((prev) =>
      prev
        .map((candidate) => (candidate.id === updated.id ? updated : candidate))
        .sort((a, b) => a.created_at.localeCompare(b.created_at)),
    );
    setError(null);
  }

  async function handleRenameThreadById(threadId: string, newTitle: string) {
    if (!stream) return;
    // The person's command: a failure is recorded, and the toast's.
    await runForPerson("Rename thread", async () => {
      await renameThread(stream.id, threadId, newTitle);
      const refreshed = await getThreadState(stream.id);
      setThreadStates((prev) => ({ ...prev, [stream.id]: refreshed }));
    });
  }

  async function handleStreamCreated(next: Stream) {
    setStreams((prev) => {
      const others = prev.filter((candidate) => candidate.id !== next.id);
      return [...others, next].sort((a, b) => a.created_at.localeCompare(b.created_at));
    });
    setError(null);
    setDaemonUnavailable(false);
    logUi("info", "stream created in ui", { streamId: next.id, title: next.title, branch: next.branch });
    // The new stream becomes the one this device shows, so a reload comes
    // back to it.
    try {
      await switchStream(next.id);
    } catch (e) {
      logUi("warn", "switch_stream after create failed", { streamId: next.id, error: String(e) });
    }
    try {
      const state = await getThreadState(next.id);
      setThreadStates((prev) => ({ ...prev, [next.id]: state }));
    } catch (e) {
      setError(String(e));
    }
    setStream(next);
  }

  async function handleSelectThread(streamId: string, threadId: string) {
    try {
      // Cross-stream selection: switch the active stream first so the
      // rest of the app (center tabs, file session, work panel) reframes
      // around the new stream before we apply the thread selection.
      // Doing this sequentially also avoids a race where handleSwitch's
      // own setThreadStates write (seeded from the prefetched state with
      // the OLD selectedThreadId) clobbers the selection we're about to
      // apply.
      if (stream && streamId !== stream.id) {
        await handleSwitch(streamId);
      }
      const next = await selectThread(streamId, threadId);
      setThreadStates((prev) => ({ ...prev, [streamId]: next }));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleCreateThread(title: string): Promise<boolean> {
    if (!stream) return false;
    // It opens on the session picker: a thread names no agent.
    const ran = await runForPerson("New thread", async () => {
      const { state: next } = await createThread(stream.id, title);
      setThreadStates((prev) => ({ ...prev, [stream.id]: next }));
    });
    return ran.ok;
  }

  async function handleMarkThread(threadId: string, mark: ThreadMark | null) {
    const owner = streams.find((s) => threadStates[s.id]?.threads.some((t) => t.id === threadId));
    if (!owner) return;
    await runForPerson(mark ? "Mark thread" : "Clear thread mark", async () => {
      const next = await markThread(owner.id, threadId, mark);
      setThreadStates((prev) => ({ ...prev, [owner.id]: next }));
    });
  }

  async function handleCloseThread(threadId: string) {
    if (!stream) return;
    await runForPerson("Close thread", async () => {
      const next = await closeThread(stream.id, threadId);
      setThreadStates((prev) => ({ ...prev, [stream.id]: next }));
    });
  }

  const currentThreadState = useMemo(
    () => (stream ? threadStates[stream.id] ?? NO_THREADS : NO_THREADS),
    [threadStates, stream],
  );
  const selectedThread = currentThreadState.threads.find((thread) => thread.id === currentThreadState.selectedThreadId) ?? null;

  return {
    streams,
    stream,
    threadStates,
    threadStatesRef,
    workspaceContext,
    currentThreadState,
    selectedThread,
    switchStream: handleSwitch,
    renameStream: handleRenameStreamById,
    renameThread: handleRenameThreadById,
    streamCreated: handleStreamCreated,
    selectThread: handleSelectThread,
    createThread: handleCreateThread,
    markThread: handleMarkThread,
    closeThread: handleCloseThread,
    /** The setters the backend's events write through
     *  (`useBackendSubscriptions`). */
    live: { setStreams, setStream, setThreadStates, setWorkspaceContext },
  };
}
