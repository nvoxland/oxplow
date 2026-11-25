import type { AgentSessionRow } from "../agentSessions.js";
import type { Stream, Thread, ThreadState } from "../api.js";
import { readThreadSessions } from "../agentSessions.js";
import { listStreams, recordUsage, resolveSymbol } from "../api.js";
import { type DiffSpec } from "../components/Diff/DiffPane.js";
import { showToast } from "../components/toastStore.js";
import { buffers } from "../editor/openFiles.js";
import { classifyExternalUrl } from "../external-url-allowlist.js";
import { logUi } from "../logger.js";
import { type EditorNavigationTarget } from "../lsp.js";
import { recordOpError } from "../opErrors.js";
import { openInBrowser } from "../openIn.js";
import { drawnByRenderer, renderersCache } from "../pages/useRenderers.js";
import { openHandlerFor, type OpenHandlers } from "../tabs/openHandlers.js";
import { openRef as openRefAt, setWindowRefOpener } from "../tabs/openRef.js";
import { diffRef, directoryRef, diskFilePath, externalUrlRef, fileOf, fileRef, fileRefAt, gitCommitRef, knowledgePageRef } from "../tabs/pageRefs.js";
import { navigate as sessionNavigate, slotShowing, type Origin } from "../tabs/session.js";
import { type TabRef } from "../tabs/tabState.js";
import { streamOfThread } from "../tabs/threadTabOps.js";
import { type Extension } from "../tauri-bridge/generated/bindings.js";
import { identityOf, keyOf, knowledgeUri, pageUri, paramOf, typeOf } from "../uri/uri.js";
import { isWorkspaceDir } from "../workspaceFiles.js";
import { useCallback, useEffect, useRef } from "react";
import type { useWindowTabs } from "./useWindowTabs.js";

export interface WindowOpenersOptions {
  stream: Stream | null;
  selectedThread: Thread | null;
  /** The shown thread's open agent sessions; null until they are read. */
  selectedThreadSessions: AgentSessionRow[] | null;
  setError: (error: string | null) => void;
  /** What a ref that names a thread or a stream does (`useWindowStreams`). */
  streams: {
    threadStatesRef: { current: Record<string, ThreadState> };
    selectThread: (streamId: string, threadId: string) => Promise<void>;
    streamCreated: (stream: Stream) => Promise<void>;
    switchStream: (streamId: string) => Promise<void>;
  };
  /** The tabs they open into (`useWindowTabs`). */
  tabs: Pick<
    ReturnType<typeof useWindowTabs>,
    "sessions" | "updateSession" | "openTab" | "closePageTab" | "setCenterActive" | "pendingSessionRef" | "setPendingSessionTick"
  >;
}

/** The window's openers: a ref of any kind to where it opens (a tab, its
 *  file, a symbol's line, an agent session or its transcript, a thread or
 *  a stream), a page in place in a tab, and closing a file with unsaved
 *  edits (with Undo). `handleOpenPageRef` always calls the latest
 *  `handleOpenPage`, for the callers made before it. Shell-neutral. */
export function useWindowOpeners({ stream, selectedThread, selectedThreadSessions, setError, streams, tabs }: WindowOpenersOptions) {
  const { threadStatesRef, selectThread: handleSelectThread, streamCreated: handleStreamCreated, switchStream: handleSwitch } = streams;
  const { sessions, updateSession, openTab, closePageTab, setCenterActive, pendingSessionRef, setPendingSessionTick } = tabs;
  const selectedThreadId = selectedThread?.id ?? null;
  // Forward ref so the forms and offers (declared above handleOpenPage) can
  // route through the same page-tab opener used by every other caller.
  // The ref is populated in a useEffect after handleOpenPage is defined.
  const handleOpenPageRef = useRef<((ref: TabRef) => void) | null>(null);

  /** Open the working-tree file `path`, its tab showing `ref` (the file in
   *  a view: `#lines=`; the plain file when none is given). Its buffer is
   *  read first, so a clicked path that isn't a file (a dotted word in
   *  prose, a stale link) opens no tab and says so; a directory opens its
   *  folder's page. With `ref` null only the buffer is read. */
  async function handleOpenFile(path: string, ref: TabRef | null = fileRef(path)) {
    if (!stream) return;
    // A trailing-slash link (terminal or agent output) is a folder: no
    // read is tried for it.
    if (path.endsWith("/") && (await isWorkspaceDir(stream.id, path))) {
      setError(null);
      handleOpenPageRef.current?.(directoryRef(path));
      return;
    }
    const buffer = await buffers.open(stream, path);
    if (buffer.state !== "ready") {
      if (await isWorkspaceDir(stream.id, path)) {
        setError(null);
        handleOpenPageRef.current?.(directoryRef(path));
        return;
      }
      if (buffer.state === "missing") {
        setError(`File not found: ${path}`);
        logUi("warn", "open file: target does not exist", { streamId: stream.id, path });
      } else {
        setError(buffer.error ?? `Couldn't open ${path}`);
        logUi("error", "failed to open file", { streamId: stream.id, path, error: buffer.error });
      }
      return;
    }
    if (!ref) return;
    openTab(ref);
    setError(null);
    void recordUsage({
      kind: "editor-file",
      key: path,
      event: "open",
      streamId: stream.id,
      threadId: selectedThread?.id ?? null,
    }).catch(() => {});
  }

  /** A `symbol:` ref opens its file at the symbol's line; one pinned to a
   *  version opens the file as it was then. A pin whose version is gone
   *  opens the current symbol and says so. */
  async function openSymbol(ref: string) {
    if (!stream) return;
    try {
      const at = await resolveSymbol(stream.id, ref);
      if (!at) {
        recordOpError({ label: "Open symbol", message: `${ref} isn't a symbol in this stream now.` });
        return;
      }
      if (at.versionGone) {
        recordOpError({ label: "Open symbol", message: `The version ${ref} pins is gone; showing the symbol as it is now.` });
      }
      if (at.revision) {
        handleOpenPage(fileRef(at.path, at.revision));
        return;
      }
      await handleNavigateToLocation({ path: at.path, line: at.line, column: at.col });
    } catch (e) {
      recordOpError({ label: "Open symbol", message: e instanceof Error ? e.message : String(e) });
    }
  }

  /** Open `target`'s file at its line: its tab in that view (`ref`, else
   *  `#lines=<line>`, where the editor lands). */
  async function handleNavigateToLocation(target: EditorNavigationTarget, ref?: TabRef) {
    await handleOpenFile(target.path, ref ?? fileRefAt(target.path, target.line));
  }

  /** Close the tab showing the working file `path`. Unsaved edits close
   *  with it, and a toast offers Undo for a while: the tab and its draft
   *  come back. */
  function handleCloseOpenFile(id: string, path: string) {
    if (!stream) return;
    const stashed = buffers.stash(stream.id, path);
    closePageTab(id);
    if (!stashed) return;
    const basename = path.split("/").pop() ?? path;
    showToast({
      message: `Closed "${basename}" with unsaved changes.`,
      actionLabel: "Undo",
      onUndo: () => {
        void buffers.restore(stashed).then(() => openTab(fileRef(path)));
      },
    });
  }

  const closeDiffTab = (id: string) => closePageTab(id);

  const handleOpenKnowledge = useCallback((store: string, id: string) => {
    const tid = selectedThread?.id ?? null;
    if (!tid) return;
    openTab(knowledgePageRef(store, id));
    const sid = stream?.id ?? null;
    if (sid) {
      void recordUsage({
        kind: "knowledge",
        key: knowledgeUri(store, id),
        event: "open",
        streamId: sid,
        threadId: tid,
      }).catch(() => {});
    }
  }, [stream?.id, selectedThread?.id, openTab]);

  /**
   * Open an http(s) URL as an in-app sandboxed external-url tab.
   * Validates through the scheme allowlist; a URL it refuses opens in
   * the system browser, so the person still gets to follow the link.
   */
  const handleOpenExternalUrl = useCallback((rawUrl: string) => {
    const verdict = classifyExternalUrl(rawUrl);
    if (!verdict.ok) {
      openInBrowser(rawUrl);
      return;
    }
    handleOpenPageRef.current?.(externalUrlRef(verdict.url));
  }, []);

  /** Open the commit page for a wikilink-resolved commit SHA. */
  const handleOpenCommit = useCallback((sha: string) => {
    if (!sha) return;
    handleOpenPageRef.current?.(gitCommitRef(sha));
  }, []);

  const handleOpenPage = useCallback((ref: TabRef) => {
    // Every kind not handled below opens as a per-thread page tab.
    const openAsPageTab = () => openTab(ref);
    // Page-visit recording lives in the central activation effect
    // below — it fires whenever `effectiveCenterActive` resolves to a
    // new TabRef, regardless of which handler caused the activation.
    const handlers: OpenHandlers = {
      "oxplow/agent_session": (ref) => {
        // Its tab is there while it's open on this thread
        // (`reconcileSessionTabs`). A closed session, or another thread's,
        // has none: its transcript opens instead, with Reopen on it.
        const sessionId = paramOf(ref.id, "id");
        // The transcript page takes the session's bare id (`ses3`).
        const transcript = () =>
          handleOpenPageRef.current?.({ id: pageUri("session-transcript", { session: sessionId ?? keyOf(ref.id) ?? ref.id }) });
        if (sessionId && selectedThreadSessions?.some((candidate) => candidate.id === sessionId)) {
          setCenterActive(ref.id);
        } else if (sessionId && selectedThread) {
          // A session opened a moment ago (a form's `open_after`) isn't in
          // the window's list yet: read the model, and show its tab once the
          // list has it; a closed one, or another thread's, shows its transcript.
          void readThreadSessions(selectedThread.id).then(({ sessions: open }) => {
            if (open.some((candidate) => candidate.id === sessionId)) {
              pendingSessionRef.current = sessionId;
              setPendingSessionTick((tick) => tick + 1);
            }
            else transcript();
          });
        } else {
          transcript();
        }
      },
      "oxplow/symbol": (ref) => {
        void openSymbol(identityOf(ref.id) ?? ref.id);
      },
      "oxplow/file": (ref) => {
        const payload = fileOf(ref);
        if (!payload?.path) return;
        // The working tree's file is read first (a path that isn't one
        // opens no tab); the line it lands on is its view (`#lines=`).
        if (diskFilePath(ref.id) !== null) {
          void handleOpenFile(payload.path, ref);
          return;
        }
        // A file at a revision, or on another filesystem (the running
        // extensions'), reads itself.
        openTab(ref);
      },
    };
    // A page opened again in another view (a diff at another line) is the
    // tab already showing it, in that view.
    (openHandlerFor(handlers, typeOf(ref.id)) ?? openAsPageTab)(ref);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [handleOpenFile, handleOpenKnowledge, openTab, selectedThreadId, selectedThreadSessions, setCenterActive, stream?.id]);

  /**
   * Open the ref `ref` (of any kind) where its kind's page says
   * (`tabs/openRef.ts`, by way of `refOpener`): a tab through `open`, the
   * thread or stream it names, or an error. The one path every link to a
   * ref takes — a page's links (`PageNavigation.openRef`), the rail and
   * palette (the window's opener), a search hit. Kept in a ref so the
   * page contexts built in a memo always call the latest.
   */
  const openRefRef = useRef<(ref: string, how: import("../tabs/openRef.js").OpenHow, open: import("../tabs/openRef.js").RefHost["open"], closePage?: () => void) => void>(() => {});
  const extensionsRef = useRef<Extension[] | null>(null);
  openRefRef.current = (ref, how, open, closePage) => {
    void openRefAt(ref, how, {
      // The daemon is asked which page draws the tab before the tab is
      // there, so it usually has an answer when it first draws.
      open: (tab, h) => {
        if (extensionsRef.current && drawnByRenderer(tab.id)) {
          void renderersCache.prefetch(extensionsRef.current, tab.id, stream?.id ?? null);
        }
        open(tab, h);
      },
      closePage,
      showThread: async (thread) => {
        const streamId = streamOfThread(threadStatesRef.current, thread);
        if (!streamId) return false;
        await handleSelectThread(streamId, thread);
        return true;
      },
      showStream: async (id) => {
        if (!threadStatesRef.current[id]) {
          // A stream just made (New Stream) isn't in the window's state yet:
          // read it and take it in as the create flow always did.
          const made = (await listStreams()).find((candidate) => candidate.id === id);
          if (!made) return false;
          await handleStreamCreated(made);
          return true;
        }
        if (stream?.id !== id) await handleSwitch(id);
        return true;
      },
      failed: (message) => recordOpError({ label: "Open a reference", message }),
    });
  };
  useEffect(() => {
    setWindowRefOpener((ref, how) => openRefRef.current(ref, how, (tab) => handleOpenPageRef.current?.(tab)));
    return () => setWindowRefOpener(null);
  }, []);

  /** Navigate to a unified-search hit: wherever its ref opens. */
  const openSearchHit = useCallback((hit: import("../api.js").SearchHit) => {
    openRefRef.current(hit.uri, { newTab: true }, (tab) => handleOpenPage(tab));
  }, [handleOpenPage]);


  /** The collection a page is opened from, as an entry keeps it. */
  const originOf = (origin: Origin | undefined, ref: TabRef): Origin | undefined => {
    if (!origin) return undefined;
    // A live origin has no items; it finds its place when it is resolved.
    if (origin.live) return origin;
    const at = origin.items.findIndex((item) => item.ref.id === ref.id);
    const index = at !== -1 ? at : origin.index;
    if (index < 0 || index >= origin.items.length) return undefined;
    return { ...origin, index };
  };

  /**
   * Browser-style in-tab navigation: the tab showing `currentTabId` goes
   * to `ref`, pushed (Back returns) or in its place with `replace`. A tab
   * id no tab shows opens `ref` in a new tab.
   */
  const handleNavigateInTab = useCallback((
    currentTabId: string,
    ref: TabRef,
    origin?: Origin,
    replace = false,
  ) => {
    if (!selectedThreadId) return;
    const slot = slotShowing(sessions[selectedThreadId] ?? { tabs: [], active: null, mru: [], next: 1 }, currentTabId);
    // A symbol isn't a page (it opens its file at its line), and an agent
    // session has its own tab: replacing, the page they replace closes.
    const resource = typeOf(ref.id);
    if (resource === "oxplow/symbol" || resource === "oxplow/agent_session" || !slot) {
      if (replace && slot) closePageTab(currentTabId);
      handleOpenPage(ref);
      return;
    }
    updateSession(selectedThreadId, (session) =>
      sessionNavigate(session, slot, { ref, from: originOf(origin, ref) }, { replace }),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [closePageTab, handleOpenPage, selectedThreadId, sessions, updateSession]);

  /** Open a diff in the tab showing `currentTabId`; Back returns to the
   *  page it came from. */
  const handleOpenDiffInTab = useCallback((currentTabId: string, spec: DiffSpec, origin?: Origin) => {
    handleNavigateInTab(currentTabId, diffRef(spec), origin);
  }, [handleNavigateInTab]);

  // Keep the forward ref in sync with the latest handleOpenPage. Used by
  // the forms and offers (declared above handleOpenPage) so menu/keyboard
  // dispatches route through the same page-tab opener.
  useEffect(() => {
    handleOpenPageRef.current = handleOpenPage;
  }, [handleOpenPage]);

  return {
    closeDiffTab,
    extensionsRef,
    handleCloseOpenFile,
    handleNavigateInTab,
    handleNavigateToLocation,
    handleOpenCommit,
    handleOpenDiffInTab,
    handleOpenExternalUrl,
    handleOpenFile,
    handleOpenKnowledge,
    handleOpenPage,
    handleOpenPageRef,
    openRefRef,
    openSearchHit,
  };
}
