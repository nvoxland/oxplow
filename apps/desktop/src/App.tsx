import { useResourcesLoader } from "./resources.js";
import { useShellKind } from "./shells/shellChoice.js";
import { shellMenu } from "./shells/shellMenu.js";
import type { ShellWindow } from "./shells/shellWindow.js";
import { TabletShell } from "./shells/TabletShell.js";
import { useEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";
import {
  type AgentStatus,
  type AgentStatusEntry,
  desktopBridge,
  probeDaemon,
  runCommand,
  type SqlCell,
} from "./api.js";
import { buildMenuBar, buildNativeMenuSnapshots, goUriFromMenuId, INSTALL_CLI_ID, menuItemById, OPEN_RECENT_PREFIX, viewMenu, type GoRow } from "./menuBar.js";
import { GoMenuFeed } from "./components/Panels/GoMenuFeed.js";
import { RailMenuFeed, type RailMenu } from "./components/Panels/RailMenuFeed.js";
import { Navigator } from "./components/Navigator.js";
import { DEVICES_MENU_ID, DevicesSlideover } from "./host/DevicesSlideover.js";
import { StatusBar } from "./components/StatusBar.js";
import { Dock } from "./components/Dock/Dock.js";
import { TitleBar } from "./components/TitleBar.js";
import { showToast } from "./components/toastStore.js";
import { UndoToastStack } from "./components/UndoToast.js";
import { paramOf, typeOf } from "./uri/uri.js";
import { nextMode, openWithOf } from "./pages/renderers.js";
import { drawnByRenderer } from "./pages/useRenderers.js";
import { PersonCommandConfirm } from "./components/PersonCommandConfirm.js";
import { RemoteConnectionBanner } from "./components/RemoteConnectionBanner.js";
import { useBackendSubscriptions } from "./useBackendSubscriptions.js";
import { Menubar } from "./components/Menubar.js";
import { CenterTabs } from "./components/CenterTabs/CenterTabs.js";
import { RailHud } from "./components/RailHud/RailHud.js";
import { NarrowRail } from "./components/RailHud/NarrowRail.js";
import { useNarrowWindow } from "./windowWidth.js";
import { useWindowStreams } from "./window/useWindowStreams.js";
import { WindowProviders } from "./window/WindowProviders.js";
import { useWindowTabs } from "./window/useWindowTabs.js";
import { useWindowOpeners } from "./window/useWindowOpeners.js";
import { useWindowHost } from "./window/useWindowHost.js";
import { useWindowCommands } from "./window/useWindowCommands.js";
import { changeView, TabLoading, useWindowPages } from "./window/useWindowPages.js";
import type { TabRef } from "./tabs/tabState.js";
import { liveItemsOf, resolvedOrigin } from "./tabs/liveOrigin.js";
import { useBookmarks } from "./tabs/bookmarks.js";
import { sessionLabel } from "./agentKinds.js";
import { useAgentHarnesses } from "./useAgentHarnesses.js";
import { useThreadSessions } from "./agentSessions.js";
import { rollUpAgentStatus, threadStatuses } from "./agentStatusRollup.js";
import { useExtensions } from "./extensionsStore.js";
import { foundationDown, foundationDownText } from "./foundationDown.js";
import { diskFilePath,  refFromTabId, recordedPageKind,  newStreamRef, alertsRef,  streamSettingsRef, threadSettingsRef, viewParamsOf,  pageTabRef, terminalRef } from "./tabs/pageRefs.js";
import { setSelectedThread } from "./contextBasket.js";
import { recordOpError, setOpErrorContext } from "./opErrors.js";
import { installContextMenuSuppressor } from "./context-menu.js";
import { QuickOpenOverlay } from "./components/QuickOpenOverlay.js";
import { NON_TRACKED_KINDS } from "./components/RailHud/history.js";
import {
  current as currentEntry,
  identityOf as pageIdentityOf,
} from "./tabs/session.js";
import { installCli, listRecentProjects, shellAvailable } from "./api.js";
import { triggerRemoteResync } from "./api.js";
import type { RecentProjectView } from "./tauri-bridge/generated/bindings.js";
import { finishPageVisit, recordPageVisit } from "./api.js";
import { PageVisitTimer } from "./pageVisitTimer.js";
import { advanceDaemonProbeState, INITIAL_DAEMON_PROBE_STATE } from "./daemon-recovery.js";
import { personCommands } from "./personCommands.js";
import { logUi, setUiLogContext } from "./logger.js";
import { startKnowledgeStores } from "./knowledgeStores.js";
import { setOpenInHost } from "./openIn.js";
import { setGitMenuHost, type GitStatusEntry } from "./gitMenu.js";
import { openCompareWith, openRollback } from "./components/Git/gitDialogStore.js";
import { GitDialogs } from "./components/Git/GitDialogs.js";
import { startRecordStores } from "./recordStoresLoader.js";
import { copyText } from "./clipboard.js";

export function App() {
  // A window under 800 px wide gives the rail up so the page keeps the room.
  const narrowWindow = useNarrowWindow();
  const harnesses = useAgentHarnesses();

  // Each agent session's status (`sessionStatusKey`). A thread's dot is its
  // sessions' roll-up, and its question (the rail dot's tooltip) the one
  // the awaiting session asks.
  const [sessionStatuses, setSessionStatuses] = useState<Record<string, AgentStatusEntry>>({});
  const { statuses: agentStatuses, questions: agentQuestions } = useMemo(
    () => threadStatuses(Object.values(sessionStatuses)),
    [sessionStatuses],
  );
  const [error, setError] = useState<string | null>(null);
  // Oxplow ▸ Devices… (the app menu; the shell handles its id).
  const [devicesOpen, setDevicesOpen] = useState(false);
  // Markdown reads `[[<store>:<id>]]` for the stores of every shared model:
  // know them from the start.
  useEffect(() => {
    startKnowledgeStores();
    startRecordStores();
  }, []);
  // The error banner auto-dismisses; it's transient feedback (e.g. a
  // mistargeted file link), not a persistent state the user must clear.
  useEffect(() => {
    if (!error) return;
    const t = setTimeout(() => setError(null), 8000);
    return () => clearTimeout(t);
  }, [error]);
  const [daemonUnavailable, setDaemonUnavailable] = useState(false);
  // The streams, their threads and the shown thread; loaded on mount and
  // on every reconnect.
  const {
    streams,
    stream,
    threadStates,
    threadStatesRef,
    workspaceContext,
    selectedThread,
    switchStream: handleSwitch,
    renameStream: handleRenameStreamById,
    renameThread: handleRenameThreadById,
    streamCreated: handleStreamCreated,
    selectThread: handleSelectThread,
    createThread: handleCreateThread,
    markThread: handleMarkThread,
    closeThread: handleCloseThread,
    live: windowStreamsLive,
  } = useWindowStreams({ setError, setDaemonUnavailable });
  const [quickOpenVisible, setQuickOpenVisible] = useState(false);
  const [quickOpenQuery, setQuickOpenQuery] = useState("");
  const daemonDownLogged = useRef(false);
  const daemonProbeState = useRef(INITIAL_DAEMON_PROBE_STATE);
  // macOS uses the native top-of-screen menu bar (driven by the
  // renderer through `setNativeMenu`, built in
  // src-tauri/.../commands/menu.rs); the in-window Menubar would
  // duplicate it, so it only renders off-Mac.
  const isMac = typeof navigator !== "undefined"
    && /Mac|iPhone|iPad|iPod/.test(navigator.platform || navigator.userAgent || "");


  useEffect(() => {
    let cancelled = false;

    async function check() {
      const alive = await probeDaemon();
      if (cancelled) return;
      const decision = advanceDaemonProbeState(daemonProbeState.current, alive);
      daemonProbeState.current = decision.next;
      if (decision.refresh) {
        // Daemon came back after an HTTP-level outage. Resync the stores
        // in place instead of a full page reload (which drops unsaved
        // editor drafts). triggerRemoteResync re-runs the same reconnect
        // handlers a WS recovery would.
        logUi("info", "daemon recovered, resyncing ui");
        setDaemonUnavailable(false);
        triggerRemoteResync();
        return;
      }
      setDaemonUnavailable(decision.next.unavailable);
      if (decision.next.unavailable && !daemonDownLogged.current) {
        logUi("warn", "daemon probe failed");
        daemonDownLogged.current = true;
      }
      if (alive) {
        daemonDownLogged.current = false;
      }
    }

    check();
    const timer = window.setInterval(check, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);



  // The selected thread's open agent sessions: its session tabs follow them.
  const selectedThreadSessions = useThreadSessions(selectedThread?.id ?? null);
  const selectedThreadId = selectedThread?.id ?? null;
  // The window's tabs: every thread's session of tabs, the shown thread's
  // active tab and the operations on its tabs.
  const {
    sessions,
    sessionsLoaded,
    updateSession,
    sessionsKept,
    pageTitles,
    setPageTitle,
    dirtyTabs,
    tabSavesRef,
    tabFindsRef,
    editingSetterFor,
    tabActions,
    actionsSetterFor,
    centerActive,
    pageTabsForActiveThread,
    effectiveCenterActive,
    shownFilePath,
    setCenterActive,
    openTab,
    pendingSessionRef,
    setPendingSessionTick,
    handleReorderCenterTabs,
    openingThread,
    closePageTab,
    liveItems,
    setLiveItems,
    handleStepOrigin,
    handleGoBack,
    handleGoForward,
  } = useWindowTabs({ selectedThreadId, stream, threadStates, selectedThreadSessions });
  // Where a ref opens: its tab, its file, a thread or a stream.
  const {
    closeDiffTab,
    extensionsRef,
    handleCloseOpenFile,
    handleNavigateInTab,
    handleOpenDiffInTab,
    handleOpenExternalUrl,
    handleOpenFile,
    handleOpenPage,
    handleOpenPageRef,
    openRefRef,
    openSearchHit,
  } = useWindowOpeners({
    stream,
    selectedThread,
    selectedThreadSessions,
    setError,
    streams: { threadStatesRef, selectThread: handleSelectThread, streamCreated: handleStreamCreated, switchStream: handleSwitch },
    tabs: { sessions, updateSession, openTab, closePageTab, setCenterActive, pendingSessionRef, setPendingSessionTick },
  });

  // A failed operation from no named thread is the one on screen's.
  useEffect(() => {
    setOpErrorContext({ thread: selectedThreadId, stream: stream?.id ?? null });
  }, [selectedThreadId, stream?.id]);
  useEffect(() => {
    setSelectedThread(selectedThreadId, stream?.id ?? null);
  }, [selectedThreadId, stream?.id]);

  const streamStatuses = useMemo<Record<string, AgentStatus>>(() => {
    const out: Record<string, AgentStatus> = {};
    for (const s of streams) {
      const threads = threadStates[s.id]?.threads ?? [];
      // The threads' roll-up: what you owe (an answer, a dead turn) shows
      // even from a collapsed rail, ahead of work in flight.
      out[s.id] = rollUpAgentStatus(threads.flatMap((t) => agentStatuses[t.id] ?? [])) ?? "waiting";
    }
    return out;
  }, [streams, threadStates, agentStatuses]);

  // Backend-event subscription wiring (workspace context, backlog, task
  // events, followup/thread/stream changes, config, agent status) lives
  // in this hook so App doesn't carry ~10 inline subscription effects.
  // Extensions' ref kinds (P8.D7): icons, labels, routes and wikilinks.
  useResourcesLoader();
  useBackendSubscriptions({
    threadStatesRef,
    ...windowStreamsLive,
    setSessionStatuses,
  });



  // Feed the stall watchdog (logger.ts) the coarse "what's on screen"
  // context so a `main thread stalled` WARN names the active page + the
  // open file, not just a duration.
  useEffect(() => {
    setUiLogContext({
      streamId: stream?.id ?? null,
      centerActive: effectiveCenterActive,
      filePath: shownFilePath,
    });
  }, [stream?.id, effectiveCenterActive, shownFilePath]);

  // Central page-visit recorder. Fires whenever the resolved active
  // tab changes — covers new-tab opens (any handler), switching to an
  // already-open tab, and snap-back when the active tab closes. Skips
  // the first commit so persisted-tab restoration on startup doesn't
  // flood History.
  //
  // The effect's dependency list is intentionally minimal: only the
  // active id + selected thread should re-fire it. The supporting
  // context (page tabs, file open-order, tasks titles, stream id)
  // is read through a ref so e.g. opening a SECOND tab doesn't
  // re-record a visit for the FIRST (still-active) one.
  // tabLabelByIdRef is the single source of "what's this tab called
  // right now" — populated by the centerTabs builder after it has
  // applied any pageTitles override. The history recorder reads from
  // this so RailHud History shows the *same* string as the tab strip
  // for every kind.
  const tabLabelByIdRef = useRef<Record<string, string>>({});
  const visitContextRef = useRef<{
    pageTabs: TabRef[];
    streamId: string | null;
  }>({ pageTabs: [], streamId: null });
  visitContextRef.current = {
    pageTabs: pageTabsForActiveThread,
    streamId: stream?.id ?? null,
  };
  const hydratedHistoryRef = useRef(false);
  // What was last recorded: a page whose view changed (a filter, a row
  // picked) is the same visit, so Recents lists the page once.
  const lastVisitRef = useRef<string | null>(null);
  // How long each recorded visit was shown (pageVisitTimer.ts): it ends when
  // another page is shown, the thread changes or the window goes away.
  const visitTimerRef = useRef<PageVisitTimer | null>(null);
  visitTimerRef.current ??= new PageVisitTimer(
    (id, ms) => void finishPageVisit(id, ms).catch(() => {}),
    undefined,
    document.visibilityState === "visible",
  );
  useEffect(() => {
    const end = () => visitTimerRef.current?.end();
    window.addEventListener("pagehide", end);
    return () => window.removeEventListener("pagehide", end);
  }, []);
  useEffect(() => {
    const shownRef = visitContextRef.current.pageTabs.find((t) => t.id === effectiveCenterActive);
    const visit = `${selectedThreadId}|${pageIdentityOf(shownRef?.id ?? effectiveCenterActive)}`;
    const same = lastVisitRef.current === visit;
    lastVisitRef.current = visit;
    if (!hydratedHistoryRef.current) {
      hydratedHistoryRef.current = true;
      return;
    }
    if (same) return;
    const timer = visitTimerRef.current;
    timer?.end();
    if (!selectedThreadId) return;
    const ctx = visitContextRef.current;
    const ref = ctx.pageTabs.find((t) => t.id === effectiveCenterActive) ?? null;
    if (!ref || NON_TRACKED_KINDS.has(recordedPageKind(ref))) return;
    const shown = timer?.begin();
    // Defer briefly so the freshly-activated page's `usePageTitle`
    // effect fires and the centerTabs memo applies it before we
    // snapshot the label. tabLabelByIdRef is what the tab strip
    // currently displays — reading from it keeps the two surfaces
    // consistent; there is no separate fallback derivation.
    const deferred = setTimeout(() => {
      const ctxAfter = visitContextRef.current;
      const label = tabLabelByIdRef.current[effectiveCenterActive] ?? ref.id;
      // A visit that couldn't be recorded (the window reloading, the daemon
      // away) is not worth an unhandled rejection.
      void recordPageVisit({
        refKind: recordedPageKind(ref),
        // A visit is of the page, whatever its view.
        refId: pageIdentityOf(ref.id),
        label,
        streamId: ctxAfter.streamId,
        threadId: selectedThreadId,
      })
        .then((id) => shown?.identify(id))
        .catch(() => {});
    }, 250);
    return () => clearTimeout(deferred);
  }, [effectiveCenterActive, selectedThreadId]);





  // Open In ▸ (`openIn.ts`): the stream's worktree, oxplow's web tab, and a
  // new terminal at a directory, shown once it is open.
  useEffect(() => {
    if (!stream) return;
    const streamId = stream.id;
    setOpenInHost({
      worktree: stream.worktree_path,
      openInApp: (url) => handleOpenExternalUrl(url),
      openRef: (ref) => {
        const tab = refFromTabId(ref);
        if (tab) handleOpenPageRef.current?.(tab);
      },
      openTerminalAt: (dir) =>
        void runCommand("oxplow.terminal.open", { stream: streamId, dir })
          .then(({ result }) => {
            // The Terminal page picks its shell by title, in its view: the
            // one tab shows the new terminal.
            const title = (result as { title?: string } | null)?.title;
            handleOpenPageRef.current?.(title ? pageTabRef("terminal", undefined, { terminal: title }) : terminalRef());
          })
          .catch((e: unknown) => recordOpError({ label: "Open a terminal", message: e instanceof Error ? e.message : String(e) })),
    });
    // Git ▸ (`gitMenu.ts`): the stream's changes, read as a menu opens, and
    // its dialogs (`GitDialogs`).
    setGitMenuHost({
      status: () =>
        runCommand("oxplow.vcs.status", { stream_id: streamId }).then(
          ({ result }) => ((result as { entries?: GitStatusEntry[] } | null)?.entries ?? []),
        ),
      openRef: (ref) => {
        const tab = refFromTabId(ref);
        if (tab) handleOpenPageRef.current?.(tab);
      },
      compareWith: openCompareWith,
      rollback: openRollback,
      ignore: (path) =>
        void runCommand("oxplow.git.ignore", { stream: streamId, entry: path }).catch((e: unknown) =>
          recordOpError({ label: "Add to .gitignore", message: e instanceof Error ? e.message : String(e) }),
        ),
    });
    return () => {
      setOpenInHost(null);
      setGitMenuHost(null);
    };
  }, [stream, handleOpenExternalUrl]);







  // What the navigation keys do now: the active tab's Back, Forward and
  // step through its origin. Read at the key press, so the handler below
  // needn't rebind as tabs move.
  const navKeysRef = useRef({ back: () => {}, forward: () => {}, step: (_by: number) => {}, mode: () => {} });
  navKeysRef.current = (() => {
    const session = selectedThreadId ? sessions[selectedThreadId] : undefined;
    const tab = session?.tabs.find((t) => t.slot === session.active);
    if (!tab || !selectedThreadId) return { back: () => {}, forward: () => {}, step: () => {}, mode: () => {} };
    const thread = selectedThreadId;
    const entry = currentEntry(tab);
    const origin = entry.from ? resolvedOrigin(entry.from, liveItemsOf(liveItems, tab.slot, entry.from), entry.ref) : null;
    return {
      back: () => handleGoBack(tab.slot),
      forward: () => handleGoForward(tab.slot),
      step: (by: number) => {
        if (origin) handleStepOrigin(tab.slot, origin.index + by);
      },
      // The next of the pages that can draw the URI, the nav bar's pick.
      mode: () => {
        const ref = entry.ref;
        const openWith = drawnByRenderer(ref.id)
          ? openWithOf(renderersOf(ref.id), viewParamsOf(ref), (patch) =>
              updateSession(thread, (s) =>
                changeView(s, tab.slot, (now) => {
                  const next: Record<string, SqlCell> = { ...now };
                  for (const [k, v] of Object.entries(patch)) {
                    if (v === undefined) delete next[k];
                    else next[k] = v;
                  }
                  return next;
                })))
          : undefined;
        if (openWith) openWith.pick(nextMode(openWith));
      },
    };
  })();





  const extensions = useExtensions();
  extensionsRef.current = extensions;
  // The window as the daemon's host, and its tab report for agents.
  const { windowHandlersRef, windowVisible } = useWindowHost({
    stream,
    streams,
    threadStates,
    selectedThreadId,
    selectedThreadSessions,
    extensions,
    tabs: { sessions, sessionsLoaded, updateSession, sessionsKept, closePageTab, centerActive, setCenterActive, tabSavesRef, tabFindsRef, openingThread },
    openers: { handleOpenPageRef, handleNavigateInTab },
    switchStream: handleSwitch,
    selectThread: handleSelectThread,
    view: { effectiveCenterActive, visitContextRef },
    openQuickOpen: (query) => {
      setQuickOpenQuery(query);
      setQuickOpenVisible(true);
    },
  });
  useEffect(() => visitTimerRef.current?.setVisible(windowVisible), [windowVisible]);
  // The window's commands: what their `when` reads, the person's offers,
  // and the keyboard shortcuts that run them.
  const { liveTerminals, offerDeps, offers } = useWindowCommands({
    stream,
    selectedThread,
    workspaceContext,
    harnesses,
    tabs: { centerActive, dirtyTabs, shownFilePath },
    handleOpenPageRef,
    windowHandlersRef,
    navKeysRef,
  });
  const [recentProjects, setRecentProjects] = useState<RecentProjectView[]>([]);
  // The native menu and its recent projects are the shell's: a browser
  // window has neither to ask for.
  useEffect(() => {
    if (!shellAvailable()) return;
    listRecentProjects()
      .then(setRecentProjects)
      .catch((e) => logUi("warn", "failed to load recent projects for menu", { error: String(e) }));
  }, []);
  // Cancel the native WKWebView context menu everywhere except inputs,
  // contenteditable, Monaco, and the terminal (see context-menu.ts).
  useEffect(() => installContextMenuSuppressor(), []);
  // The menu bar (File, Edit) and what search lists: the bus's offers,
  // and the shell's project commands.
  // View ▸ Sidebar Panels is the viewer's rail panels (`RailMenuFeed`, inside
  // the layout's provider, hands them up as the layout changes).
  const [railMenu, setRailMenu] = useState<RailMenu>({ panels: [], toggle: () => {} });
  const menuGroups = useMemo(
    () => [...buildMenuBar(offers), viewMenu(railMenu.panels, railMenu.toggle)],
    [offers, railMenu],
  );
  // The stars the thread sees (a page's star says where it is starred).
  const bookmarks = useBookmarks(selectedThreadId, stream?.id ?? null);
  // The Go menu is what the Go To pane shows (`GoMenuFeed`, inside the
  // panels' provider, hands it up as the pane's run changes).
  const [goRows, setGoRows] = useState<GoRow[]>([]);
  const nativeMenuSnapshots = useMemo(
    () => buildNativeMenuSnapshots(menuGroups, recentProjects, goRows),
    [menuGroups, recentProjects, goRows],
  );


  useEffect(() => {
    if (!shellAvailable()) return;
    void desktopBridge().setNativeMenu(nativeMenuSnapshots).catch((error) => {
      logUi("error", "failed to update native menu", { error: String(error) });
    });
  }, [nativeMenuSnapshots]);

  useEffect(() => {
    return desktopBridge().onMenuCommand((commandId: string) => {
      if (commandId === DEVICES_MENU_ID) {
        setDevicesOpen(true);
        return;
      }
      // Dynamic "Open Recent ▸ <project>" entries aren't menu items:
      // `oxplow.project.open`'s operation opens the trailing path in a
      // new window. A recent whose `.oxplow/` has since been deleted
      // errors rather than re-creating it.
      if (commandId.startsWith(OPEN_RECENT_PREFIX)) {
        const path = commandId.slice(OPEN_RECENT_PREFIX.length);
        void Promise.resolve(
          windowHandlersRef.current["projects.write"]?.open?.({ path, new_window: true }, { threadId: null, actor: "human", call: null }),
        ).catch((e: unknown) => {
          recordOpError({
            label: "Open project",
            message: e instanceof Error ? e.message : String(e),
          });
        });
        return;
      }
      const goUri = goUriFromMenuId(commandId);
      if (goUri !== null) {
        const ref = refFromTabId(goUri);
        if (ref) flushSync(() => handleOpenPageRef.current?.(ref));
        return;
      }
      if (commandId === INSTALL_CLI_ID) {
        installCli().then(
          (done) => showToast({ message: `Installed ${done.path}. ${done.hint ?? ""}`.trim() }),
          (e: unknown) => recordOpError({ label: "Install command line", message: e instanceof Error ? e.message : String(e) }),
        );
        return;
      }
      const command = menuItemById(menuGroups, commandId);
      if (!command || !command.run) return;
      // React 18 only auto-flushes effects synchronously for discrete
      // user input events (click, keydown on webContents). IPC messages
      // from the main process don't qualify, so setState calls made in
      // this callback stay queued until the next real input event wakes
      // the scheduler — users reported menu dispatches stalling 10+
      // seconds. flushSync commits inside the callback. The commands
      // that open modals (plan.newTask, etc.) additionally go
      // through an imperative ref registered by the target pane so the
      // modal setState also commits here rather than via useEffect.
      const run = command.run;
      flushSync(() => { void run(); });
    });
  }, [menuGroups]);
  const foundationErrors = foundationDown(extensions);
  // The shown thread's tabs, as every shell draws them.
  const { windowTabs: centerTabs, renderersOf } = useWindowPages({
    stream,
    selectedThread,
    selectedThreadSessions,
    extensions,
    sessionStatuses,
    harnesses,
    liveTerminals,
    offerDeps,
    bookmarks,
    tabs: {
      sessions,
      updateSession,
      openTab,
      closePageTab,
      handleGoBack,
      handleGoForward,
      handleStepOrigin,
      liveItems,
      pageTitles,
      setPageTitle,
      dirtyTabs,
      tabActions,
      editingSetterFor,
      actionsSetterFor,
    },
    openers: { handleOpenPage, handleNavigateInTab, handleOpenDiffInTab, openRefRef },
    tabLabelByIdRef,
  });

  // A tab's close, for every shell: a file's tab closes its file, a diff's
    // its diff, an agent session's the session (its toast offers Undo), any
    // other page its tab.
    const closeTab = (id) => {
    const path = diskFilePath(id);
    if (path !== null) handleCloseOpenFile(id, path);
    else if (id.startsWith("diff:")) closeDiffTab(id);
    else if (typeOf(id) === "oxplow/agent_session") {
      // Closing a session's tab closes the session at once, its
      // toast offering Undo (it reopens on the saved
      // conversation); its tab goes when its row closes.
      const ref = refFromTabId(id);
      const sessionId = ref ? paramOf(ref.id, "id") : null;
      if (sessionId) {
        const session = selectedThreadSessions?.find((c) => c.id === sessionId);
        const name = session ? session.displayName || sessionLabel(harnesses, session) : "session";
        void personCommands.run(`Close ${name}`, "oxplow.agent_session.close", {
          session: sessionId,
        });
      }
    }
    else closePageTab(id);
  };
  
  const shell = useShellKind();
  // The window as a shell draws it (`shells/shellWindow.ts`).
  const shellWindow: ShellWindow = {
    streams,
    stream,
    threadStates,
    selectedThread,
    streamStatuses,
    agentStatuses,
    agentQuestions,
    switchStream: handleSwitch,
    selectThread: handleSelectThread,
    createThread: async (streamId, title) => {
      if (streamId !== stream?.id) await handleSwitch(streamId);
      return handleCreateThread(title);
    },
    renameStream: handleRenameStreamById,
    renameThread: handleRenameThreadById,
    markThread: handleMarkThread,
    closeThread: handleCloseThread,
    openStreamSettings: (streamId) => handleOpenPage(streamSettingsRef(streamId)),
    openThreadSettings: (threadId) => handleOpenPage(threadSettingsRef(threadId)),
    openNewStream: () => handleOpenPage(newStreamRef()),
    tabs: centerTabs,
    activeTabId: effectiveCenterActive,
    activateTab: (id) => setCenterActive(id),
    closeTab,
    openPage: handleOpenPage,
    openSearch: () => setQuickOpenVisible(true),
    // View is the rail's panels; a shell with no rail draws its own.
    menu: shellMenu(menuGroups.filter((g) => g.id !== "view")),
    tray: <Dock streamId={stream?.id ?? null} onOpenPage={handleOpenPage} />,
  };
  // The window's errors, a line above the page in every shell.
  const errorBar = (
    <div style={{ borderBottom: error ? "1px solid var(--border)" : undefined, flexShrink: 0 }}>
      {error ? (
        <div
          onClick={() => setError(null)}
          title="Click to dismiss"
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: 8,
            padding: "2px 12px",
            background: "var(--bg-2)",
            color: "#ff6b6b",
            fontSize: 11,
            minHeight: 22,
            borderBottom: "1px solid var(--border)",
            cursor: "pointer",
          }}
        >
          <span>{error}</span>
          <span aria-hidden style={{ opacity: 0.7, paddingLeft: 8 }}>
            ✕
          </span>
        </div>
      ) : null}
    </div>
  );

  return (
    <WindowProviders
      streamId={stream?.id ?? null}
      threadId={selectedThreadId}
      session={sessions[selectedThreadId ?? ""]}
      onLiveItems={(slot, key, items) => setLiveItems((prev) => ({ ...prev, [slot]: { key, items } }))}
      onReviewAlerts={(open) => handleOpenPage((open ? refFromTabId(open) : null) ?? alertsRef())}
    >
    <GoMenuFeed onRows={setGoRows} />
    <RailMenuFeed onChange={setRailMenu} />
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", overflow: "hidden" }}>
      {shell === "tablet" ? (
        <>
          {errorBar}
          <DevicesSlideover open={devicesOpen} onClose={() => setDevicesOpen(false)} />
          <TabletShell win={shellWindow} />
        </>
      ) : (
        <>
      {/* macOS uses an Overlay titlebar (transparent, hidden title), so the
          webview reaches the top edge: the title bar is the window's top,
          its empty space drags it and its start leaves room for the
          floating traffic lights. Elsewhere it sits under the menu bar. */}
      {!isMac ? <Menubar groups={menuGroups} /> : null}
      <TitleBar
        stream={stream}
        thread={selectedThread ? { id: selectedThread.id, title: selectedThread.title } : null}
        vcsEnabled={workspaceContext.vcsEnabled}
        leftInset={isMac ? 78 : 10}
        onOpenSearch={() => setQuickOpenVisible(true)}
      />
      {errorBar}
      <DevicesSlideover open={devicesOpen} onClose={() => setDevicesOpen(false)} />
      <div style={{ flex: 1, display: "flex", flexDirection: "row", minHeight: 0, minWidth: 0 }}>
        <Navigator
          streams={streams}
          currentStreamId={stream?.id ?? null}
          threadStates={threadStates}
          streamStatuses={streamStatuses}
          agentStatuses={agentStatuses}
          agentQuestions={agentQuestions}
          onSwitchStream={handleSwitch}
          onSelectThread={handleSelectThread}
          onCreateThread={async (streamId, title) => {
            if (streamId !== stream?.id) await handleSwitch(streamId);
            return handleCreateThread(title);
          }}
          onOpenNewStreamPage={() => handleOpenPage(newStreamRef())}
          onRenameStream={handleRenameStreamById}
          onRenameThread={handleRenameThreadById}
          onMarkThread={handleMarkThread}
          onCloseThread={handleCloseThread}
          onOpenStreamSettings={(streamId) => handleOpenPage(streamSettingsRef(streamId))}
          onOpenThreadSettings={(threadId) => handleOpenPage(threadSettingsRef(threadId))}
          vcsEnabled={workspaceContext.vcsEnabled}
        />
        <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0, minWidth: 0, background: "var(--surface-chrome)" }}>
        <div style={{ flex: 1, display: "flex", flexDirection: "row", minHeight: 0, minWidth: 0, position: "relative" }}>
        <NarrowRail narrow={narrowWindow}>
          {(closeRail) => (
            <RailHud
              streamId={stream?.id ?? null}
              onOpenPage={(ref) => {
                closeRail();
                handleOpenPage(ref);
              }}
            />
          )}
        </NarrowRail>
        <div style={{
          flex: 1,
          display: "flex",
          flexDirection: "column",
          minHeight: 0,
          minWidth: 0,
          overflow: "hidden",
          // The main content area floats as a panel on the chrome: the
          // left side is curved (revealing chrome), the whole right side
          // stays flush/square against the window edge. The border is
          // drawn on the panel itself so the corners stay closed
          // regardless of tab state.
          marginLeft: narrowWindow ? 0 : 6,
          border: "1px solid var(--border-strong)",
          borderTopLeftRadius: 8,
          borderBottomLeftRadius: 8,
          // Git ▸'s dialogs dock over it (`GitDialogs`).
          position: "relative",
        }}>
          {stream ? (
            <CenterTabs
              key={stream.id}
              tabs={centerTabs}
              activeId={effectiveCenterActive}
              onActivate={(id) => setCenterActive(id)}
              onClose={closeTab}
              onReorder={handleReorderCenterTabs}
            />
          ) : <TabLoading />}
          {stream ? (
            <GitDialogs
              streamId={stream.id}
              openRef={(ref) => {
                const tab = refFromTabId(ref);
                if (tab) handleOpenPageRef.current?.(tab);
              }}
            />
          ) : null}
        </div>
        </div>
        <Dock streamId={stream?.id ?? null} onOpenPage={handleOpenPage}>
          <StatusBar />
        </Dock>
        </div>
      </div>
        </>
      )}
      <QuickOpenOverlay
        open={quickOpenVisible}
        initialQuery={quickOpenQuery}
        stream={stream}
        threadId={selectedThreadId}
        selectedFilePath={shownFilePath}
        offers={offers}
        onClose={() => setQuickOpenVisible(false)}
        onOpenFile={(path) => {
          void handleOpenFile(path);
        }}
        onOpenPage={(ref) => {
          handleOpenPage(ref);
        }}
        onOpenSearchHit={openSearchHit}
      />
      {daemonUnavailable ? <DaemonDownDialog /> : null}
      {!daemonUnavailable && foundationErrors ? (
        <FoundationDownDialog key={foundationErrors.join("\n")} errors={foundationErrors} />
      ) : null}
      <UndoToastStack />
      <PersonCommandConfirm />
      <RemoteConnectionBanner />
    </div>
    </WindowProviders>
  );
}

/**
 * Shown when the required foundation extension isn't running. The deploy
 * command is foundation's own, so the shell can't run it: the dialog shows
 * the backend's reason it is held. Dismissal is local; a new problem text (the key)
 * shows it again.
 */
function FoundationDownDialog({ errors }: { errors: string[] }) {
  const [dismissed, setDismissed] = useState(false);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setDismissed(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  if (dismissed) return null;
  const lines = foundationDownText(errors);
  const button = { padding: "8px 14px", borderRadius: 4, cursor: "pointer", fontFamily: "inherit" } as const;
  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0, 0, 0, 0.65)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 999,
        padding: 24,
      }}
    >
      <div
        style={{
          width: "min(560px, 100%)",
          background: "var(--bg-2)",
          border: "1px solid var(--border)",
          borderRadius: 8,
          padding: 20,
          boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 12px 40px rgba(0, 0, 0, 0.4)",
        }}
      >
        <div style={{ fontSize: 18, fontWeight: 600, marginBottom: 8 }}>{lines[0]}</div>
        <div style={{ color: "var(--muted)", lineHeight: 1.5, marginBottom: 12 }}>{lines[1]}</div>
        <ul style={{ margin: "0 0 12px", paddingLeft: 20, lineHeight: 1.5 }}>
          {errors.map((line, i) => (
            <li key={i}>{line}</li>
          ))}
        </ul>
        <div style={{ marginBottom: 16 }}>{lines[lines.length - 1]}</div>
        <div style={{ display: "flex", gap: 8 }}>
          <button
            type="button"
            onClick={() => void copyText(lines.join("\n"))}
            style={{ ...button, background: "var(--accent)", color: "#fff", border: "none" }}
          >
            Copy
          </button>
          <button
            type="button"
            onClick={() => setDismissed(true)}
            style={{ ...button, background: "transparent", color: "inherit", border: "1px solid var(--border)" }}
          >
            Dismiss
          </button>
        </div>
      </div>
    </div>
  );
}

function DaemonDownDialog() {
  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0, 0, 0, 0.65)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 1000,
        padding: 24,
      }}
    >
      <div
        style={{
          width: "min(520px, 100%)",
          background: "var(--bg-2)",
          border: "1px solid var(--border)",
          borderRadius: 8,
          padding: 20,
          boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 12px 40px rgba(0, 0, 0, 0.4)",
        }}
      >
        <div style={{ fontSize: 18, fontWeight: 600, marginBottom: 8 }}>Backend daemon disconnected</div>
        <div style={{ color: "var(--muted)", lineHeight: 1.5, marginBottom: 16 }}>
          The backend daemon was killed or is no longer reachable. Stream switching, terminal panes, and hook
          updates will not keep working until the daemon is started again.
        </div>
        <button type="button"
          onClick={() => window.location.reload()}
          style={{
            background: "var(--accent)",
            color: "#fff",
            border: "none",
            padding: "8px 14px",
            borderRadius: 4,
            cursor: "pointer",
            fontFamily: "inherit",
          }}
        >
          Reload after restart
        </button>
      </div>
    </div>
  );
}
