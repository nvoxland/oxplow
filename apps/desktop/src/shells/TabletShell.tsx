import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";
import { Menu as MenuGlyph, Search, X } from "lucide-react";

import { AgentStatusDot } from "../components/AgentStatusDot.js";
import { BranchChip } from "../components/TitleBar.js";
import { buildTabMenu } from "../components/CenterTabs/tabMenu.js";
import { moveToIndex } from "../components/CenterTabs/centerTabsReorder.js";
import type { MenuItem } from "../menu.js";
import { TabBodies } from "../components/CenterTabs/TabBodies.js";
import { useContextMenu, useRowContextMenu, type MenuItems } from "../components/useRowContextMenu.js";
import type { WindowTab } from "../window/windowTab.js";
import type { ShellWindow } from "./shellWindow.js";
import { RemoveStreamDialog } from "../components/RemoveStreamDialog.js";
import { listProjects, setHold } from "../host/hostApi.js";
import { hostOrigin, hostToken } from "../tauri-bridge/transport.js";
import { useStreamThreadMenus } from "../window/useStreamThreadMenus.js";
import { readWindowContext } from "../windowContext.js";
import { TabletSwitcher, type SwitcherHost } from "./TabletSwitcher.js";

/** The host that served this page, for the switcher's projects; none in the
 *  Mac app or without a device token. */
function pageHost(): SwitcherHost | null {
  const origin = hostOrigin();
  const token = hostToken();
  if (!origin || !token) return null;
  return {
    origin,
    token,
    current: readWindowContext()?.project ?? null,
    listProjects: (h, t) => listProjects(h, t),
    setHold: async (h, t, id, held) => void (await setHold(h, t, id, held)),
    go: (path) => window.location.assign(path),
  };
}

/**
 * The tablet's shell: one line at the top (the context: stream and thread,
 * search, and the thread's tabs), the page at full width, and the panels'
 * tray at the bottom. No rail, no Navigator, no menu bar: a finger's
 * surfaces only. It draws the window's model (`ShellWindow`) as the desktop
 * layout does; what it does not draw yet is listed in `shellContract.ts`.
 */
export function TabletShell({ win, host: given }: { win: ShellWindow; host?: SwitcherHost | null }) {
  // The page's host, made once: a new object each render would read the
  // projects again on every render while the switcher is open.
  const [pageHostOnce] = useState(pageHost);
  const host = given === undefined ? pageHostOnce : given;
  const [switching, setSwitching] = useState(false);
  const commands = useContextMenu();
  // The stream and thread menus and their flows: this shell has no
  // Navigator, so they are drawn in the switcher, which a flow opens.
  const menus = useStreamThreadMenus({
    streams: win.streams,
    threadStates: win.threadStates,
    streamStatuses: win.streamStatuses,
    currentStreamId: win.stream?.id ?? null,
    onRenameStream: win.renameStream,
    onRenameThread: win.renameThread,
    onMarkThread: win.markThread,
    onCloseThread: win.closeThread,
    onOpenStreamSettings: win.openStreamSettings,
    onOpenThreadSettings: win.openThreadSettings,
    reveal: () => setSwitching(true),
  });
  const shown = win.tabs.filter((t) => !t.hidden);
  // Other threads whose agent waits on the person: the switcher shows which.
  const waitingElsewhere = Object.entries(win.agentStatuses).filter(([id, status]) => status === "awaiting" && id !== win.selectedThread?.id).length;
  const active = win.tabs.find((t) => t.id === win.activeTabId) ?? shown[0] ?? null;
  const bulkClosable = shown.map((t) => ({ id: t.id, closable: t.closable && !t.pinned }));
  return (
    <div data-testid="tablet-shell" data-dock-column="" style={frame}>
      <div style={topLine}>
        <button type="button" data-testid="tablet-context" style={contextButton} title="Projects, streams and threads" aria-haspopup="dialog" onClick={() => setSwitching(true)}>
          <span style={{ fontWeight: 600 }}>{win.stream?.title ?? "…"}</span>
          {win.selectedThread ? <span style={{ color: "var(--text-secondary)" }}>› {win.selectedThread.title}</span> : null}
          {waitingElsewhere > 0 ? (
            <span data-testid="tablet-context-waiting" className="ox-badge" title={`${waitingElsewhere} other thread${waitingElsewhere === 1 ? "" : "s"} waiting on you`}>
              {waitingElsewhere} waiting
            </span>
          ) : null}
        </button>
        {win.stream ? <BranchChip stream={win.stream} vcsEnabled={win.vcsEnabled} /> : null}
        <div role="tablist" aria-label="Tabs" data-testid="tablet-tabs" style={strip}>
          {shown.map((tab) => (
            <TabletTab
              key={tab.id}
              tab={tab}
              active={tab.id === active?.id}
              menu={() => withMoves(buildTabMenu({ tab, bulkClosable, activeId: active?.id ?? "", onClose: win.closeTab, onActivate: win.activateTab }), tab, shown, win.reorderTabs)}
              onActivate={() => win.activateTab(tab.id)}
              onClose={() => win.closeTab(tab.id)}
            />
          ))}
        </div>
        <button type="button" className="ox-icon-button" aria-label="Search" data-testid="title-bar-search" onClick={win.openSearch}>
          <Search size={16} aria-hidden />
        </button>
        {/* The menu bar's commands: Save, Find, the extensions' menu items. */}
        <button
          type="button"
          className="ox-icon-button"
          aria-label="Menu"
          aria-haspopup="menu"
          data-testid="tablet-menu"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            // Continue from ▸ is read as the menu opens; without it (none,
            // or the read failed) the menu is the rest.
            const items = win
              .continueFrom()
              .then((devices) => (devices.length > 0 ? [...win.menu, { id: "shell.menu.continue", label: "Continue from", enabled: true, submenu: devices }] : win.menu))
              .catch(() => win.menu);
            commands.openAt({ x: r.left, y: r.bottom + 2 }, items);
          }}
        >
          <MenuGlyph size={16} aria-hidden />
        </button>
      </div>
      <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
        <div style={pageArea}>
          {/* Per stream: its threads' tab keys repeat across streams. */}
          <TabBodies key={win.stream?.id ?? ""} tabs={win.tabs} activeId={active?.id ?? null} />
          {win.gitDialogs}
        </div>
        {win.aside}
      </div>
      <div data-testid="tablet-tray" style={trayLine}>
        {win.tray}
      </div>
      <TabletSwitcher
        open={switching}
        onClose={() => {
          setSwitching(false);
          menus.setRenaming(null);
          menus.setPendingNewThreadFor(null);
        }}
        win={win}
        menus={menus}
        host={host}
      />
      <RemoveStreamDialog removal={menus.removal} />
      {menus.requestedMenu}
      {commands.menu}
    </div>
  );
}

/** A finger cannot drag a tab: its menu moves it a place left or right
 *  (Move Left / Move Right), never before a pinned session, as the desktop
 *  strip's drag does. */
function withMoves(items: MenuItem[] | Promise<MenuItem[]>, tab: WindowTab, shown: WindowTab[], reorder: (ids: string[]) => void): MenuItem[] | Promise<MenuItem[]> {
  const movable = (t: WindowTab | undefined) => !!t && t.closable && !t.pinned;
  if (!movable(tab)) return items;
  const ids = shown.map((t) => t.id);
  const at = ids.indexOf(tab.id);
  const moves: MenuItem[] = [
    { id: "tab.move-sep", label: "", enabled: false, separator: true },
    { id: "tab.move-left", label: "Move Left", enabled: movable(shown[at - 1]), run: () => reorder(moveToIndex(ids, tab.id, at - 1)) },
    { id: "tab.move-right", label: "Move Right", enabled: movable(shown[at + 1]), run: () => reorder(moveToIndex(ids, tab.id, at + 2)) },
  ];
  return items instanceof Promise ? items.then((list) => [...list, ...moves]) : [...items, ...moves];
}

/** One tab of the strip: a tap shows it, its × closes it, and a long press
 *  (or the Menu key) opens the window's tab menu. */
function TabletTab({ tab, active, menu, onActivate, onClose }: { tab: WindowTab; active: boolean; menu(): MenuItems; onActivate(): void; onClose(): void }) {
  const { menu: menuNode, openAt: _openAt, ...menuHandlers } = useRowContextMenu(menu, tab.label);
  // The shown tab is in view: a portrait strip fits few.
  const ref = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (active) ref.current?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [active]);
  return (
    <div
      ref={ref}
      role="tab"
      tabIndex={0}
      aria-selected={active}
      data-testid={`tablet-tab-${tab.id}`}
      title={tab.hint ?? tab.label}
      style={{ ...tabStyle, ...(active ? activeTab : null) }}
      onClick={onActivate}
      {...menuHandlers}
    >
      {tab.agentStatus ? <AgentStatusDot status={tab.agentStatus} question={tab.question} /> : null}
      <span style={tabLabel}>
        {tab.dirty ? "● " : ""}
        {tab.label}
      </span>
      {/* An agent session's tab too (pinned, closable): its × closes the
          session, as the desktop strip's does. */}
      {tab.closable ? (
        <button
          type="button"
          className="ox-icon-button"
          aria-label={`Close ${tab.label}`}
          data-testid={`tablet-tab-close-${tab.id}`}
          onClick={(e) => {
            e.stopPropagation();
            onClose();
          }}
        >
          <X size={14} aria-hidden />
        </button>
      ) : null}
      {menuNode}
    </div>
  );
}

const frame: CSSProperties = { display: "flex", flexDirection: "column", flex: 1, minHeight: 0, overflow: "hidden", background: "var(--surface-chrome)" };
const topLine: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: "var(--space-2)",
  minHeight: "var(--hit-min)",
  padding: "0 var(--space-2)",
  borderBottom: "1px solid var(--border-strong)",
  flexShrink: 0,
};
const contextButton: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 6,
  minHeight: "var(--hit-min)",
  padding: "0 var(--space-2)",
  background: "transparent",
  border: "none",
  color: "var(--text-primary)",
  fontSize: "var(--text-sm)",
  flexShrink: 0,
  maxWidth: "35vw",
  overflow: "hidden",
  whiteSpace: "nowrap",
  textOverflow: "ellipsis",
};
const strip: CSSProperties = { display: "flex", alignItems: "stretch", gap: 2, flex: 1, minWidth: 0, overflowX: "auto" };
const tabStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 6,
  minHeight: "var(--hit-min)",
  padding: "0 var(--space-2)",
  borderRadius: 6,
  color: "var(--text-secondary)",
  fontSize: "var(--text-sm)",
  cursor: "pointer",
  flexShrink: 0,
  maxWidth: 240,
};
const activeTab: CSSProperties = { background: "var(--surface-app)", color: "var(--text-primary)" };
const tabLabel: CSSProperties = { overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" };
const pageArea: CSSProperties = { flex: 1, minHeight: 0, minWidth: 0, display: "flex", flexDirection: "column", position: "relative", background: "var(--surface-app)" };
const trayLine: CSSProperties = { flexShrink: 0, borderTop: "1px solid var(--border-strong)" };
