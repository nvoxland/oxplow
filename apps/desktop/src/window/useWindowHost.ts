import type { Extension } from "../tauri-bridge/generated/bindings.js";
import type { AgentSessionRow } from "../agentSessions.js";
import type { Stream, ThreadState } from "../api.js";
import { insertIntoAgent } from "../agent-input-bus.js";
import { desktopBridge, forgetQueuedTabs, getCurrentStream, getThreadState, onRemoteReconnect, openInSystemBrowser, openProject, reportTabs, subscribeOxplowEvents, listQueuedTabs } from "../api.js";
import { getUiClientId } from "../uiClientId.js";
import { startClientHost, type ClientCallContext, type ClientHandlers } from "../clientHost.js";
import { copyText } from "../clipboard.js";
import { buffers } from "../editor/openFiles.js";
import { windowPageRuns } from "../pageRuns.js";
import { planNavigation } from "../tabs/navigation.js";
import { getPageDetailStore } from "../tabs/openPageDetail.js";
import { diskFilePath, refFromTabId } from "../tabs/pageRefs.js";
import { activeRef, close as sessionClose, open as sessionOpen, slotShowing } from "../tabs/session.js";
import { buildTabReport, createTabReporter } from "../tabs/tabReport.js";
import { streamOfThread } from "../tabs/threadTabOps.js";
import { useTabReportRepeats } from "../tabs/useTabReportRepeats.js";
import { typeOf } from "../uri/uri.js";
import { useEffect, useMemo, useRef, useState } from "react";
import { filesAreLocal, shellAvailable } from "../tauri-bridge/transport.js";

/** The calls a window takes from its daemon, less what acts on this Mac when
 *  the daemon is on another machine: a Mac window on a remote host would
 *  otherwise open a folder here, open the Mac's browser or write its
 *  clipboard because that host asked (the "a person's, not an agent's"
 *  check is the daemon's, which is that host's). A browser page keeps them:
 *  its browser bounds them. Pure; exported for tests. */
export function hostedHere(handlers: ClientHandlers, where: { shell: boolean; local: boolean }): ClientHandlers {
  if (!where.shell || where.local) return handlers;
  const { "projects.write": _projects, ...rest } = handlers;
  const show = rest["window.show"];
  if (!show) return rest;
  const { open_url: _url, copy_text: _copy, ...shown } = show;
  return { ...rest, "window.show": shown };
}
import { pickAndCreateProject, pickAndOpenProject } from "./projectPickers.js";
import type { useWindowOpeners } from "./useWindowOpeners.js";
import type { useWindowTabs } from "./useWindowTabs.js";

/** How often an unchanged window repeats its tab report (the backend forgets
 *  one that falls silent for five minutes). */
const TAB_REPORT_HEARTBEAT_MS = 60_000;

export interface WindowHostOptions {
  stream: Stream | null;
  streams: Stream[];
  threadStates: Record<string, ThreadState>;
  selectedThreadId: string | null;
  /** The shown thread's open agent sessions; null until they are read. */
  selectedThreadSessions: AgentSessionRow[] | null;
  /** The extensions that say what draws a page; null until loaded. */
  extensions: Extension[] | null;
  tabs: Pick<
    ReturnType<typeof useWindowTabs>,
    | "sessions"
    | "sessionsLoaded"
    | "updateSession"
    | "sessionsKept"
    | "closePageTab"
    | "centerActive"
    | "setCenterActive"
    | "tabSavesRef"
    | "tabFindsRef"
    | "openingThread"
  >;
  openers: Pick<ReturnType<typeof useWindowOpeners>, "handleOpenPageRef" | "handleNavigateInTab">;
  switchStream: (streamId: string) => Promise<void>;
  selectThread: (streamId: string, threadId: string) => Promise<void>;
  /** What the shell shows: the active tab it draws and its visit context. */
  view: {
    effectiveCenterActive: string;
    visitContextRef: { current: { pageTabs: { id: string }[] } };
  };
  /** The shell's Quick Open, with a query. */
  openQuickOpen: (query: string) => void;
}

/** The window as the daemon's host (`clientHost.ts`): what only a window
 *  can do for a command the daemon runs over it (an agent's `oxplow.tab.*`
 *  in its own thread, save, find, navigate, copy, open a project), taken
 *  once the window is ready; and the tab report that tells agents what it
 *  shows (`tabs/tabReport.ts`), with its focus and visibility. */
export function useWindowHost({
  stream,
  streams,
  threadStates,
  selectedThreadId,
  selectedThreadSessions,
  extensions,
  tabs,
  openers,
  switchStream: handleSwitch,
  selectThread: handleSelectThread,
  view,
  openQuickOpen,
}: WindowHostOptions) {
  const { sessions, sessionsLoaded, updateSession, sessionsKept, closePageTab, centerActive, setCenterActive, tabSavesRef, tabFindsRef, openingThread } = tabs;
  const { handleOpenPageRef, handleNavigateInTab } = openers;
  const { effectiveCenterActive, visitContextRef } = view;
  // Read when a call comes, so the handlers needn't be made again for it.
  const openQuickOpenRef = useRef(openQuickOpen);
  openQuickOpenRef.current = openQuickOpen;
  // Report what this window shows — the thread it shows, its focus and
  // visibility, the shown page's published detail (a page's params) and
  // its page runs — so an agent's `list_tabs` / `get_open_page` see what
  // the person sees. The tabs themselves are not reported: they are the
  // stored sessions (`tab_session`, saved at once by `useTabSessions`). Best-effort and debounced; see tabs/tabReport.ts and
  // tabs/openPageDetail.ts.
  const [pageDetailTick, setPageDetailTick] = useState(0);
  useEffect(() => getPageDetailStore().subscribe(() => setPageDetailTick((t) => t + 1)), []);
  const [pageRunsTick, setPageRunsTick] = useState(0);
  useEffect(() => windowPageRuns().subscribeRunIds(() => setPageRunsTick((t) => t + 1)), []);
  const [windowFocused, setWindowFocused] = useState(() => document.hasFocus());
  const [windowVisible, setWindowVisible] = useState(() => document.visibilityState === "visible");
  useEffect(() => {
    const focus = () => setWindowFocused(true);
    const blur = () => setWindowFocused(false);
    const visibility = () => setWindowVisible(document.visibilityState === "visible");
    window.addEventListener("focus", focus);
    window.addEventListener("blur", blur);
    document.addEventListener("visibilitychange", visibility);
    return () => {
      window.removeEventListener("focus", focus);
      window.removeEventListener("blur", blur);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, []);
  // The last thing told, repeated while nothing changes: the backend forgets
  // a window that stops reporting (`client_reports::FORGET_AFTER_MS`), and an
  // idle window with tabs open is still there.
  const lastTabReportRef = useRef<ReturnType<typeof buildTabReport> | null>(null);
  const tabReporter = useMemo(
    () => createTabReporter((report) => reportTabs(report).then((answer) => windowPageRuns().reopen(answer.missingRuns))),
    [],
  );
  useTabReportRepeats(lastTabReportRef, tabReporter, () => windowPageRuns().runIds(), TAB_REPORT_HEARTBEAT_MS);
  useEffect(() => {
    const timer = setTimeout(() => {
      const ctx = visitContextRef.current;
      const shown = ctx.pageTabs.find((t) => t.id === effectiveCenterActive) ?? null;
      const report = buildTabReport({
        runs: windowPageRuns().runIds(),
        focused: windowFocused,
        visible: windowVisible,
        shownThread: selectedThreadId ?? null,
        detail: shown ? getPageDetailStore().get(shown.id) : null,
      });
      lastTabReportRef.current = report;
      tabReporter.send(report);
    }, 300);
    return () => clearTimeout(timer);
  }, [effectiveCenterActive, selectedThreadId, pageDetailTick, pageRunsTick, windowFocused, windowVisible, tabReporter]);

  // The window as a command host (`clientHost.ts`): what only it can do,
  // for a command the daemon runs over it — an agent's `oxplow.tab.*`, in
  // the agent's own thread (never switching the thread or stream shown).
  const windowHandlers = useMemo<ClientHandlers>(() => {
    const threadOf = (ctx: ClientCallContext) => {
      const thread = ctx.threadId ?? selectedThreadId;
      if (!thread) throw new Error("no thread is shown");
      return thread;
    };
    const refOf = (input: unknown) => {
      const id = (input as { uri?: unknown } | null)?.uri;
      if (typeof id !== "string") throw new Error("`uri` is a page's URI");
      const ref = refFromTabId(id);
      if (!ref) throw new Error(`\`${id}\` isn't a page's ref`);
      return ref;
    };
    const open = async (input: unknown, ctx: ClientCallContext, focus: boolean) => {
      const thread = threadOf(ctx);
      const ref = refOf(input);
      if (thread === selectedThreadId && focus) {
        handleOpenPageRef.current?.(ref);
        return { uri: ref.id, focused: true };
      }
      // A working-tree file is read first: a path that isn't a file
      // opens nothing.
      const path = diskFilePath(ref.id);
      const fileStream = streams.find((candidate) => candidate.id === streamOfThread(threadStates, thread));
      if (path !== null && fileStream) {
        const buffer = await buffers.open(fileStream, path);
        if (buffer.state === "missing") throw new Error(`\`${path}\` isn't a file in the working tree`);
        if (buffer.state === "error") throw new Error(buffer.error ?? `couldn't read \`${path}\``);
      }
      updateSession(thread, (session) => {
        const opened = sessionOpen(session, { ref });
        // Opened without focus, it waits behind what the thread shows.
        return focus ? opened : { ...opened, active: session.active, mru: [...session.mru, ...opened.mru.filter((m) => !session.mru.includes(m))] };
      });
      return { uri: ref.id, focused: focus };
    };
    return {
      "editor.write": {
        // The file `uri` names, else the one the thread shows.
        save: async (input, ctx) => {
          const thread = threadOf(ctx);
          const id = (input as { uri?: unknown } | null)?.uri;
          const shown = thread === selectedThreadId ? centerActive : activeRef(sessions[thread])?.id;
          const tab = typeof id === "string" ? id : shown;
          const path = tab ? diskFilePath(tab) : null;
          // The page shown that isn't a file saves through its own Save
          // (`useTabEditing`): a knowledge page's draft, say.
          const ownSave = typeof id !== "string" && thread === selectedThreadId && tab ? tabSavesRef.current[tab] : undefined;
          if (path === null && ownSave) {
            ownSave();
            return { saved: tab };
          }
          if (path === null) throw new Error(typeof id === "string" ? `\`${id}\` isn't a file in the working tree` : "nothing to save on the page shown");
          const streamId = streamOfThread(threadStates, thread);
          if (!streamId) throw new Error(`no stream has thread \`${thread}\``);
          await buffers.save(streamId, path, ctx.call);
          return { saved: path };
        },
      },
      "window.show": {
        find: () => {
          // The page shown finds in what it edits (`useTabEditing`).
          const find = tabFindsRef.current[centerActive];
          if (!find) throw new Error("nothing to find in on the page shown");
          find();
          return null;
        },
        // Past the window (the daemon refuses an agent): the person's
        // browser, their clipboard.
        open_url: async (input) => {
          const url = (input as { url?: unknown } | null)?.url;
          if (typeof url !== "string") throw new Error("`url` is a web address");
          await openInSystemBrowser(url);
          return null;
        },
        copy_text: async (input) => {
          const text = (input as { text?: unknown } | null)?.text;
          if (typeof text !== "string") throw new Error("`text` is the text to copy");
          if (!(await copyText(text))) throw new Error("The browser refused to copy");
          return { copied: text.length };
        },
        quick_open: (input) => {
          if (!stream) throw new Error("no stream is shown");
          const query = (input as { query?: unknown } | null)?.query;
          openQuickOpenRef.current(typeof query === "string" ? query : "");
          return null;
        },
        // Takes the person to a ref (`tabs/navigation.ts`): the page opens
        // and is focused in the thread it belongs to, then the window
        // switches to that stream and thread.
        navigate: async (input) => {
          const plan = await planNavigation(input, threadStates, stream?.id ?? null, undefined, {
            current: async () => (await getCurrentStream()).id,
            threads: async (id) => getThreadState(id),
          });
          const { replace, new_tab: newTab } = (input ?? {}) as { replace?: unknown; new_tab?: unknown };
          if (plan.page) {
            if (!plan.threadId) throw new Error(`stream \`${plan.streamId}\` has no thread`);
            // As a browser follows a link: the shown tab goes there (Back
            // returns, or not with `replace`); `new_tab` gives it a tab of
            // its own.
            const shownTab = activeRef(sessions[plan.threadId])?.id;
            if (newTab !== true && shownTab && plan.threadId === selectedThreadId) {
              handleNavigateInTab(shownTab, plan.page, undefined, replace === true);
            } else {
              await open({ uri: plan.page.id }, { threadId: plan.threadId, actor: "human", call: null }, true);
            }
          }
          if (stream?.id !== plan.streamId) await handleSwitch(plan.streamId);
          if (plan.selectThread && plan.threadId) await handleSelectThread(plan.streamId, plan.threadId);
          return { opened: plan.page?.id ?? null, stream: plan.streamId };
        },
      },
      "agent_terminal.write": {
        restart: async (input, ctx) => {
          if (ctx.actor !== "human") throw new Error("Only a person can restart an agent terminal.");
          const requested = (input as { uri?: string } | null)?.uri;
          const uri = requested ?? effectiveCenterActive;
          if (typeOf(uri) !== "oxplow/agent_session") throw new Error("No agent terminal is selected.");
          await desktopBridge().restartAgentTerminal(uri);
          return null;
        },
      },
      // It fills the shown thread's agent input, unsent.
      "agent_input.write": {
        draft: (input) => {
          const text = (input as { text?: unknown } | null)?.text;
          if (typeof text !== "string") throw new Error("`text` is the draft");
          insertIntoAgent(text);
          return null;
        },
      },
      // The app shell's: projects and windows.
      "projects.write": {
        create: async () => {
          await pickAndCreateProject();
          return null;
        },
        open: async (input) => {
          const { path, new_window } = (input ?? {}) as { path?: unknown; new_window?: unknown };
          if (typeof path === "string") await openProject(path, new_window === true);
          else await pickAndOpenProject(new_window === true);
          return null;
        },
      },
      "tabs.write": {
        open: (input, ctx) => open(input, ctx, false),
        focus: (input, ctx) => open(input, ctx, true),
        close: (input, ctx) => {
          const thread = threadOf(ctx);
          const ref = refOf(input);
          if (thread === selectedThreadId) {
            closePageTab(ref.id);
          } else {
            updateSession(thread, (session) => {
              const slot = slotShowing(session, ref.id);
              return slot ? sessionClose(session, slot) : session;
            });
          }
          return { uri: ref.id, closed: true };
        },
      },
    };
    // Remade as the window's state changes; Quick Open is read through
    // its ref.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedThreadId, threadStates, closePageTab, updateSession, centerActive, effectiveCenterActive, sessions, stream, streams, setCenterActive, handleNavigateInTab]);
  const windowHandlersRef = useRef(windowHandlers);
  windowHandlersRef.current = hostedHere(windowHandlers, { shell: shellAvailable(), local: filesAreLocal() });

  // The window takes the daemon's calls (an agent's or a link's navigate,
  // `clientHost.ts`) once it can do them as asked: its stream and thread
  // loaded, the thread's first tabs opened, and the extensions that say what
  // draws a page loaded. Before that a call is refused "no window is open",
  // never half done. Once ready it stays the host: a later stream switch
  // or extensions reload doesn't take it away. The page says so on `<html>`
  // (`data-oxplow-ready`), for whoever waits for the window.
  const windowReady =
    stream !== null &&
    selectedThreadId !== null &&
    selectedThreadSessions !== null &&
    sessionsLoaded &&
    sessions[selectedThreadId] !== undefined &&
    openingThread !== selectedThreadId &&
    extensions !== null;
  const [hosting, setHosting] = useState(false);
  useEffect(() => {
    if (windowReady) setHosting(true);
  }, [windowReady]);
  useEffect(() => {
    if (!hosting) return;
    return startClientHost(() => windowHandlersRef.current, undefined, undefined, () => {
      document.documentElement.dataset.oxplowReady = "true";
    });
  }, [hosting]);

  // The tabs agents opened for this device while it was away open now, in
  // their threads and without focus, as an agent's open does. They are read
  // again after a reconnect (the device was away while the socket was down)
  // and when the daemon says one was queued just after this window
  // connected. A queued row goes only once the tabs are stored
  // (`forgetQueuedTabs`): a load that dies before that reads it again, and
  // opening a page that is open changes nothing.
  const sessionsKeptRef = useRef(sessionsKept);
  sessionsKeptRef.current = sessionsKept;
  useEffect(() => {
    if (!hosting) return;
    let taking: Promise<void> | null = null;
    const takeOnce = async () => {
      const queued = await listQueuedTabs();
      if (queued.length === 0) return;
      for (const tab of queued) {
        try {
          await windowHandlersRef.current["tabs.write"]!.open!({ uri: tab.uri }, { threadId: tab.threadId, actor: "human", call: null });
        } catch {
          // A page that can't open (a file since deleted) leaves the others.
        }
      }
      await Promise.all([...new Set(queued.map((tab) => tab.threadId))].map((thread) => sessionsKeptRef.current(thread)));
      await forgetQueuedTabs(queued.map((tab) => tab.id));
    };
    // One at a time: a second take while one runs would open the same rows.
    // A wake-up that comes while one runs is not lost: the take in flight
    // may have read the queue before the row was written, so it runs once
    // more when it ends.
    let again = false;
    const take = () => {
      if (taking) {
        again = true;
        return;
      }
      taking = takeOnce()
        .catch(() => {})
        .finally(() => {
          taking = null;
          if (again) {
            again = false;
            take();
          }
        });
    };
    take();
    const stopReconnect = onRemoteReconnect(take);
    const stopEvents = subscribeOxplowEvents((event) => {
      if (event.kind === "tabsQueued" && event.client === getUiClientId()) take();
    });
    return () => {
      stopReconnect();
      stopEvents();
    };
  }, [hosting]);

  return { windowHandlersRef, windowFocused, windowVisible };
}
