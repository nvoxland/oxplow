import { TEMP_EXTENSION } from "../pages/viewActions.js";
import { withViewParams } from "../tabs/pageRefs.js";
import { navigate as sessionNavigate, type Session } from "../tabs/session.js";
import type { AgentSessionRow } from "../agentSessions.js";
import type { Stream, Thread } from "../api.js";
import type { OfferDeps } from "../commandOffers.js";
import type { Extension, HarnessListing } from "../tauri-bridge/generated/bindings.js";
import type { WindowTab } from "./windowTab.js";
import type { useWindowOpeners } from "./useWindowOpeners.js";
import type { useWindowTabs } from "./useWindowTabs.js";
import { sessionLabel } from "../agentKinds.js";
import { type AgentStatusEntry, type SqlCell } from "../api.js";
import { type DiffSpec } from "../components/Diff/DiffPane.js";
import { recordOpError } from "../opErrors.js";
import { openInForRef, withLinkOffers } from "../openIn.js";
import { MissingPage } from "../pages/MissingPage.js";
import { PageFileView } from "../pages/PageFileView.js";
import { PartNotAPage } from "../pages/PartNotAPage.js";
import { TempPage } from "../pages/TempPage.js";
import { builtInPages } from "../pages/builtInPages.js";
import { chooseRenderer, chooseRoute } from "../pages/pageFiles.js";
import { shellView, tabHint, tabLabel } from "../pages/renderers.js";
import { type SlotNav } from "../pages/slotNav.js";
import { drawnByRenderer, useRenderers } from "../pages/useRenderers.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { bookmarkedElsewhere, moveBookmark, removeBookmark, setBookmark, useBookmarks } from "../tabs/bookmarks.js";
import { liveItemsOf, resolvedOrigin } from "../tabs/liveOrigin.js";
import { fileRef, pagePayload, viewParamsOf } from "../tabs/pageRefs.js";
import { bodies, current as currentEntry, identityOf as pageIdentityOf, patchHistoryState as sessionPatchHistoryState, setTitle as sessionSetTitle, type Entry, type Origin, type Tab } from "../tabs/session.js";
import { type TabRef } from "../tabs/tabState.js";
import { type Page as PageFile } from "../tauri-bridge/generated/bindings.js";
import { identityOf, paramOf, parseUri, typeOf } from "../uri/uri.js";
import { type VisibilityScope } from "../visibilityScope.js";
import { useMemo } from "react";
export interface WindowPagesOptions {
  stream: Stream | null;
  selectedThread: Thread | null;
  /** The shown thread's open agent sessions; null until they are read. */
  selectedThreadSessions: AgentSessionRow[] | null;
  /** The extensions that say what draws a page; null until loaded. */
  extensions: Extension[] | null;
  /** Each agent session's status, by session id. */
  sessionStatuses: Record<string, AgentStatusEntry>;
  harnesses: HarnessListing[];
  /** The agent terminals running now: Restart Terminal is offered for these. */
  liveTerminals: ReadonlySet<string>;
  /** How the window runs a command (Restart Terminal). */
  offerDeps: OfferDeps;
  /** The thread's bookmarks: a page's star says where it is starred. */
  bookmarks: ReturnType<typeof useBookmarks>;
  tabs: Pick<
    ReturnType<typeof useWindowTabs>,
    | "sessions"
    | "updateSession"
    | "openTab"
    | "closePageTab"
    | "handleGoBack"
    | "handleGoForward"
    | "handleStepOrigin"
    | "liveItems"
    | "pageTitles"
    | "setPageTitle"
    | "dirtyTabs"
    | "tabActions"
    | "editingSetterFor"
    | "actionsSetterFor"
  >;
  openers: Pick<ReturnType<typeof useWindowOpeners>, "handleOpenPage" | "handleNavigateInTab" | "handleOpenDiffInTab" | "openRefRef">;
  /** Each tab's label as drawn, for the page-visit history. */
  tabLabelByIdRef: { current: Record<string, string> };
}

/** The shown thread's tabs as every shell draws them (`WindowTab`): what
 *  draws each page (a route's page file, the page that renders its URI, or
 *  foundation's fallback), each body in its navigation context (links,
 *  Back, Forward, its origin, title, view, bookmark), a Back page kept
 *  mounted but hidden, and an agent session's tab with its status. */
export function useWindowPages({
  stream,
  selectedThread,
  selectedThreadSessions,
  extensions,
  sessionStatuses,
  harnesses,
  liveTerminals,
  offerDeps,
  bookmarks,
  tabs: windowTabState,
  openers,
  tabLabelByIdRef,
}: WindowPagesOptions) {
  const {
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
  } = windowTabState;
  const { handleOpenPage, handleNavigateInTab, handleOpenDiffInTab, openRefRef } = openers;
  const selectedThreadId = selectedThread?.id ?? null;
  // Which pages can draw each tab of the thread: the daemon's list
  // (`pages/useRenderers.ts`), fetched as tabs appear and remembered.
  const rendererUris = useMemo(
    () =>
      (selectedThreadId ? (sessions[selectedThreadId]?.tabs ?? []) : []).flatMap((t) =>
        bodies(t).map((b) => b.entry.ref.id).filter(drawnByRenderer),
      ),
    [sessions, selectedThreadId],
  );
  const renderersOf = useRenderers(extensions, rendererUris, stream?.id ?? null);
  const windowTabs: WindowTab[] = useMemo(() => {
    const tabs: WindowTab[] = [];
    // Every tab is a session tab (`tabs/session.ts`): files, diffs, agent
    // sessions and pages alike. A tab keeps a body mounted per page in its
    // history (`bodies`) so Back shows a page as it was; only the current
    // one is in the strip. A file's content and unsaved edits are its
    // buffer's (`editor/openFiles.ts`), its place the session's.
    const pageTabStartIdx = tabs.length;
    const threadSession = selectedThreadId ? sessions[selectedThreadId] : undefined;
    // Tab ids must be unique across the rendered list — the active-tab
    // lookup and the close button key on them — so a page shows once.
    const renderedTabIds = new Set<string>();
    // For each rendered tab: its session tab, its entry's ref, and whether
    // it is that tab's current entry.
    const placed: { tab: Tab; ref: TabRef; entry: Entry; current: boolean }[] = [];
    // A route (`oxplow:oxplow/page?page=…`): its page file; none, a page
    // no enabled extension has.
    const routeTab = (ref: TabRef, nav: SlotNav): WindowTab | null => {
      const page = pagePayload(ref)!;
      // Until the extensions load, nothing says what draws it.
      if (extensions === null) return { id: ref.id, label: page.page, closable: true, render: () => null };
      // A shown view is a page file on `oxplow:temp`: the daemon has it, not the extensions.
      if (page.namespace === TEMP_EXTENSION) {
        return {
          id: ref.id,
          label: "View",
          closable: true,
          render: () => <TempPage id={page.page} resource={page.params} stream={stream} threadId={selectedThread?.id ?? null} onOpenPage={nav.navOpen} />,
        };
      }
      const file = chooseRoute(extensions, page);
      // A part or a query has no tab of its own: the tab says what it is
      // and, for a part, where it is drawn.
      if (file && file.kind !== "page") return { id: ref.id, label: file.title, closable: true, render: () => <PartNotAPage page={file} extensions={extensions} /> };
      if (file) return pageFileTab(file, ref, nav, page.params, null);
      return { id: ref.id, label: page.page, closable: true, render: () => <MissingPage page={page} /> };
    };
    // What draws a tab: a route's declaration, else the page that renders
    // its URI (`pages/renderers.ts`: its resource's, as the URI's view and
    // the declarations pick), else foundation's fallback, `resource`.
    const renderTab = (ref: TabRef, nav: SlotNav): WindowTab | null => {
      if (typeOf(ref.id) === "oxplow/page") return routeTab(ref, nav);
      // A symbol opens its file at its line (openSymbol); it's never a tab.
      if (typeOf(ref.id) === "oxplow/symbol") return null;
      // Until the extensions load, nothing says what renders it.
      if (extensions === null) return { id: ref.id, label: "", closable: true, render: () => null };
      // The first ask for this URI hasn't answered: a moment of nothing.
      const renderers = renderersOf(ref.id);
      if (renderers === undefined) return { id: ref.id, label: "", closable: true, render: () => <TabLoading /> };
      const file = chooseRenderer(extensions, renderers, viewParamsOf(ref));
      // A renderer's page file runs with the URI it shows and the
      // resource's own params; nothing renders it: foundation's fallback.
      if (file) return pageFileTab(file, ref, nav, { ...renderedParams(ref.id), uri: identityOf(ref.id) ?? ref.id }, ref.id);
      return builtInPages.resource({ ref, params: renderedParams(ref.id), nav });
    };
    // A page file, drawn by the page host: its run's params are the
    // entry's URI (`PageFileView`).
    const pageFileTab = (file: PageFile, ref: TabRef, nav: SlotNav, resource: Record<string, SqlCell>, renders: string | null): WindowTab => ({
      id: ref.id,
      label: tabLabel(file, resource),
      hint: tabHint(file, resource),
      closable: true,
      ...(file.pinned ? { pinned: true } : {}),
      render: () => (
        <PageFileView
          page={file}
          resource={resource}
          renders={renders}
          stream={stream}
          threadId={selectedThread?.id ?? null}
          onOpenPage={nav.navOpen}
        />
      ),
    });
    // A currently shown page owns its navigation context. The same page
    // in an earlier tab's Back history must not claim its body first:
    // links would otherwise navigate that earlier slot instead.
    const currentBodies = new Set((threadSession?.tabs ?? []).map((t) => pageIdentityOf(currentEntry(t).ref.id)));
    for (const sessionTab of threadSession?.tabs ?? []) {
      // Navigation from any of the tab's bodies (a hidden back page too)
      // moves the tab.
      const currentId = currentEntry(sessionTab).ref.id;
      const navOpen = (newRef: TabRef, opts?: import("../tabs/PageNavigationContext.js").NavigateOptions) => {
        if (opts?.newTab) handleOpenPage(newRef);
        else handleNavigateInTab(currentId, newRef, opts?.origin, opts?.replace);
      };
      const navOpenFile = (path: string, opts?: { newTab?: boolean }) => {
        if (opts?.newTab) handleOpenPage(fileRef(path));
        else handleNavigateInTab(currentId, fileRef(path));
      };
      // A diff in this tab: Back returns to the page it came from.
      const navOpenDiff = (spec: DiffSpec, origin?: Origin) => handleOpenDiffInTab(currentId, spec, origin);
      const slotNav: SlotNav = { slotId: currentId, navOpen, navOpenFile, navOpenDiff };
      for (const body of bodies(sessionTab)) {
        const ref = body.entry.ref;
        if (!body.current && currentBodies.has(pageIdentityOf(ref.id))) continue;
        if (renderedTabIds.has(ref.id)) continue;
        renderedTabIds.add(ref.id);
        const tab = renderTab(ref, slotNav);
        if (!tab) continue;
        tabs.push({ ...tab, key: body.key, ...(body.current ? {} : { hidden: true }) });
        placed.push({ tab: sessionTab, ref, entry: body.entry, current: body.current });
      }
    }
    // Wrap each page-tab render with PageNavigationContext so descendants
    // (BacklinksList, RouteLink, in-page cross-references) can navigate
    // in-tab and the Page chrome mounts its nav bar.
    for (let i = pageTabStartIdx; i < tabs.length; i++) {
      const tab = tabs[i]!;
      const { tab: sessionTab, ref, entry, current: isCurrent } = placed[i - pageTabStartIdx]!;
      const slot = sessionTab.slot;
      const tabId = tab.id;
      const currentId = currentEntry(sessionTab).ref.id;
      const stored = isCurrent ? currentEntry(sessionTab).from : undefined;
      const origin: Origin | null = stored ? resolvedOrigin(stored, liveItemsOf(liveItems, slot, stored), ref) : null;
      const innerRender = tab.render;
      const bookmarkScope = bookmarks.find((b) => b.ref.id === ref.id)?.visibilityScope ?? null;
      const elsewhere = bookmarkScope === null ? bookmarkedElsewhere(bookmarks, ref) : null;
      const registeredTitle = pageTitles[tabId] ?? (isCurrent ? currentEntry(sessionTab).title : undefined);
      const navValue: PageNavigation = {
        navigate: (newRef, opts) => {
          // A new tab with history state (a compare's two texts) is its
          // own tab; without, the tab already showing the page, or a new one.
          if (opts?.newTab && opts.state) openTab(newRef, { historyState: opts.state });
          else if (opts?.newTab) handleOpenPage(newRef);
          else handleNavigateInTab(currentId, newRef, opts?.origin, opts?.replace);
        },
        openRef: (target, how) =>
          openRefRef.current(
            target,
            how ?? {},
            (opened, h) => {
              if (h.newTab) handleOpenPage(opened);
              else handleNavigateInTab(currentId, opened, h.origin, h.replace);
            },
            () => closePageTab(currentId),
          ),
        goBack: () => handleGoBack(slot),
        goForward: () => handleGoForward(slot),
        close: () => closePageTab(currentId),
        canGoBack: sessionTab.at > 0,
        canGoForward: sessionTab.at < sessionTab.entries.length - 1,
        pinned: !!tab.pinned,
        origin,
        stepOrigin: origin ? (index: number) => handleStepOrigin(slot, index) : undefined,
        setTitle: (t: string) => {
          setPageTitle(tabId, t);
          // Kept with the entry, for the strip before this page mounts again.
          if (isCurrent && selectedThreadId) updateSession(selectedThreadId, (session) => sessionSetTitle(session, slot, t));
        },
        title: registeredTitle,
        setEditing: isCurrent ? editingSetterFor(tabId) : undefined,
        setTabActions: isCurrent ? actionsSetterFor(tabId) : undefined,
        ask: parseUri(ref.id) ? { ref: ref.id } : undefined,
        bookmark: {
          visibilityScope: bookmarkScope,
          other: elsewhere ? { visibilityScope: elsewhere.visibilityScope } : null,
          updateOther: elsewhere
            ? () => {
                moveBookmark({ threadId: selectedThreadId, streamId: stream?.id ?? null }, elsewhere, ref, registeredTitle ?? tab.label).catch((e: unknown) =>
                  recordOpError({ label: "Bookmark", message: e instanceof Error ? e.message : String(e) }));
              }
            : undefined,
          toggle: (scope: VisibilityScope) => {
            const viewer = { threadId: selectedThreadId, streamId: stream?.id ?? null };
            const write = bookmarkScope === scope
              ? removeBookmark(viewer, ref.id)
              : setBookmark(viewer, ref, registeredTitle ?? tab.label, scope);
            write.catch((e: unknown) =>
              recordOpError({ label: "Bookmark", message: e instanceof Error ? e.message : String(e) }));
          },
        },
        ref,
        renderers: drawnByRenderer(ref.id) ? renderersOf(ref.id) : undefined,
        viewParams: viewParamsOf(ref),
        // Only the page the tab shows changes the tab; a hidden back page
        // drawing an older entry can't.
        setViewParams: isCurrent && selectedThreadId
          ? (params: Record<string, SqlCell>, opts?: { push?: boolean }) => {
              const thread = selectedThreadId;
              // A page writes its own view; the shell's keys (the renderer
              // pick) stay.
              updateSession(thread, (session) => changeView(session, slot, (now) => ({ ...shellView(now), ...params }), opts));
            }
          : undefined,
        patchViewParams: isCurrent && selectedThreadId
          ? (patch: Record<string, SqlCell | undefined>, opts?: { push?: boolean }) => {
              const thread = selectedThreadId;
              updateSession(thread, (session) =>
                changeView(session, slot, (now) => {
                  const next: Record<string, SqlCell> = { ...now };
                  for (const [k, v] of Object.entries(patch)) {
                    if (v === undefined) delete next[k];
                    else next[k] = v;
                  }
                  return next;
                }, opts),
              );
            }
          : undefined,
        historyState: entry.historyState && typeof entry.historyState === "object" && !Array.isArray(entry.historyState) ? (entry.historyState as Record<string, unknown>) : undefined,
        setHistoryState: isCurrent && selectedThreadId
          ? (patch: Record<string, unknown>) => {
              const thread = selectedThreadId;
              updateSession(thread, (session) => sessionPatchHistoryState(session, slot, patch));
            }
          : undefined,
      };
      if (registeredTitle && registeredTitle !== tab.label) {
        tab.label = registeredTitle;
      }
      if (isCurrent && dirtyTabs[tabId]) tab.dirty = true;
      // The page's actions are its tab's menu (ahead of the strip's own).
      const pageActions = isCurrent ? tabActions[tabId] : undefined;
      if (pageActions) {
        tab.contextMenu = [
          ...(tab.contextMenu ?? []),
          ...pageActions.map((a) => ({ id: a.id, label: a.label, enabled: !a.disabled, run: () => void a.run() })),
        ];
      }
      // A file's, a directory's or a web page's tab: Open In ▸ (`openIn.ts`)
      // and a file's or folder's Git ▸ (`gitMenu.ts`).
      tab.menuAtOpen = () => {
        const elsewhere = openInForRef(ref.id);
        return withLinkOffers(elsewhere ? [elsewhere] : [], ref.id).then((items) =>
          // The tab's page menu has the ref's other commands; the tab adds
          // only its Open In ▸ and Git ▸.
          items.filter((i) => i.id === "open-in" || i.id === "git"),
        );
      };
      // An agent session's tab: its harness's logo and its status dot.
      if (typeOf(ref.id) === "oxplow/agent_session") {
        const sessionId = paramOf(ref.id, "id") ?? "";
        const session = selectedThreadSessions?.find((candidate) => candidate.id === sessionId) ?? null;
        const status = sessionStatuses[sessionId];
        tab.pinned = true;
        tab.agentStatus = status?.status ?? "waiting";
        tab.question = status?.question;
        tab.icon = session ? harnesses.find((h) => h.id === session.harness)?.icon : undefined;
        tab.contextMenu = [
          ...(tab.contextMenu ?? []),
          {
            id: "agent_terminal.restart",
            label: "Restart Terminal",
            enabled: liveTerminals.has(ref.id),
            run: () => void offerDeps.run("Restart Terminal", "oxplow.agent_terminal.restart", { uri: ref.id }),
          },
        ];
        if (session && !registeredTitle) tab.label = session.displayName || sessionLabel(harnesses, session);
      }
      tab.render = () => (
        <PageNavigationContext.Provider value={navValue}>
          {innerRender()}
        </PageNavigationContext.Provider>
      );
    }
    // Snapshot the resolved labels for the history recorder so RailHud
    // History shows the same text the tab strip shows.
    const labelMap: Record<string, string> = {};
    for (const t of tabs) labelMap[t.id] = t.label;
    tabLabelByIdRef.current = labelMap;
    return tabs;
  }, [
    selectedThread,
    selectedThreadSessions,
    sessionStatuses,
    liveTerminals,
    harnesses,
    stream,
    selectedThreadId,
    sessions,
    updateSession,
    handleOpenPage,
    handleNavigateInTab,
    handleOpenDiffInTab,
    handleGoBack,
    handleGoForward,
    handleStepOrigin,
    closePageTab,
    pageTitles,
    setPageTitle,
    dirtyTabs,
    editingSetterFor,
    tabActions,
    offerDeps,
    actionsSetterFor,
    openTab,
    bookmarks,
    extensions,
    renderersOf,
  ]);

  return { windowTabs, renderersOf };
}

/** What a tab shows while the daemon says which page draws it. */
export function TabLoading() {
  return <div style={{ padding: "8px 12px", color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>Loading…</div>;
}

/** The params a tab's label and renderer are given: an oxplow URI's, and for
 *  a web page (no oxplow URI) the page itself as `uri`. */
function renderedParams(id: string): Record<string, string> {
  return parseUri(id)?.params ?? { uri: id };
}

/** `slot`'s page in the view `view` makes of its current one (its id's
 *  `#`): the same page, so a filter replaces the entry and a pick
 *  (`push`) adds one Back returns from. */
export function changeView(
  session: Session,
  slot: string,
  view: (now: Record<string, SqlCell>) => Record<string, SqlCell>,
  opts?: { push?: boolean },
): Session {
  const tab = session.tabs.find((t) => t.slot === slot);
  if (!tab) return session;
  const ref = currentEntry(tab).ref;
  return sessionNavigate(session, slot, { ref: withViewParams(ref, view(viewParamsOf(ref))) }, { replace: !opts?.push });
}
