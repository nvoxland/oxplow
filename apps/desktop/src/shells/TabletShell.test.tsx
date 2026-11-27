import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { Stream, Thread } from "../api.js";
import type { WindowTab } from "../window/windowTab.js";
import { SHELL_CONTRIBUTIONS, SHELL_PLACES } from "./shellContract.js";
import type { ShellWindow } from "./shellWindow.js";
import { TabletShell } from "./TabletShell.js";

afterEach(cleanup);

const tab = (id: string, over: Partial<WindowTab> = {}): WindowTab => ({
  id,
  label: id.toUpperCase(),
  closable: true,
  render: () => <p>body {id}</p>,
  ...over,
});

function win(over: Partial<ShellWindow> = {}): ShellWindow & { log: string[] } {
  const log: string[] = [];
  const noop = () => {};
  return {
    streams: [],
    stream: { id: "str1", title: "Main", branch: "main" } as unknown as Stream,
    vcsEnabled: true,
    threadStates: {},
    selectedThread: { id: "thr1", title: "Writer" } as unknown as Thread,
    streamStatuses: {},
    agentStatuses: {},
    agentQuestions: {},
    switchStream: noop,
    selectThread: noop,
    createThread: async () => true,
    renameStream: noop,
    renameThread: noop,
    markThread: noop,
    closeThread: noop,
    openStreamSettings: noop,
    openThreadSettings: noop,
    openNewStream: noop,
    tabs: [tab("session", { pinned: true, agentStatus: "working" }), tab("a"), tab("b")],
    activeTabId: "a",
    activateTab: (id) => log.push(`activate ${id}`),
    closeTab: (id) => log.push(`close ${id}`),
    reorderTabs: (ids) => log.push(`order ${ids.join(",")}`),
    continueFrom: async () => [],
    aside: <div data-testid="the-aside" />,
    gitDialogs: <div data-testid="the-git-dialogs" />,
    statusBar: <div data-testid="the-status-bar" />,
    openPage: noop,
    openSearch: () => log.push("search"),
    menu: [{ id: "shell.menu.file", label: "File", enabled: true, submenu: [{ id: "save", label: "Save", enabled: true, run: () => void log.push("save") }] }],
    tray: <div data-testid="the-tray" />,
    log,
    ...over,
  };
}

test("the context line names the stream and thread; the page is the active tab's", () => {
  const view = render(<TabletShell win={win()} />);
  expect(view.getByTestId("tablet-shell").textContent).toContain("main");
  expect(view.getByTestId("tablet-context").textContent).toContain("Main");
  expect(view.getByTestId("tablet-context").textContent).toContain("Writer");
  expect(view.getByTestId("center-tab-body-a").style.display).toBe("flex");
  expect(view.queryByTestId("center-tab-body-b")).toBeNull();
});

test("a tap shows a tab, its × closes it, an agent session's too", () => {
  const w = win();
  const view = render(<TabletShell win={w} />);
  fireEvent.click(view.getByTestId("tablet-tab-b"));
  fireEvent.click(view.getByTestId("tablet-tab-close-a"));
  fireEvent.click(view.getByTestId("tablet-tab-close-session"));
  expect(w.log).toEqual(["activate b", "close a", "close session"]);
});

test("a tab's menu is the window's tab menu", () => {
  const w = win();
  const view = render(<TabletShell win={w} />);
  fireEvent.contextMenu(view.getByTestId("tablet-tab-a"), { clientX: 10, clientY: 10 });
  fireEvent.click(view.getByTestId("menu-item-tab.close-others"));
  expect(w.log).toEqual(["close b"]);
});

// A finger cannot drag a tab: its menu moves it a place left or right,
// past no pinned session.
test("a tab's menu moves it left or right", () => {
  const w = win();
  const view = render(<TabletShell win={w} />);
  fireEvent.contextMenu(view.getByTestId("tablet-tab-a"), { clientX: 10, clientY: 10 });
  expect(view.getByTestId("menu-item-tab.move-left").hasAttribute("disabled")).toBe(true);
  fireEvent.click(view.getByTestId("menu-item-tab.move-right"));
  expect(w.log).toEqual(["order session,b,a"]);
});

test("the git dialogs and the status bar are drawn", () => {
  const view = render(<TabletShell win={win()} />);
  expect(view.getByTestId("the-git-dialogs")).not.toBeNull();
  expect(view.getByTestId("the-aside")).not.toBeNull();
});

test("the context line counts the other threads waiting on the person", () => {
  const view = render(<TabletShell win={win({ agentStatuses: { thr1: "awaiting", thr2: "awaiting", thr3: "working" } })} />);
  expect(view.getByTestId("tablet-context-waiting").textContent).toBe("1 waiting");
});

test("search and the tray are drawn", () => {
  const w = win();
  const view = render(<TabletShell win={w} />);
  fireEvent.click(view.getByTestId("title-bar-search"));
  expect(w.log).toEqual(["search"]);
  expect(view.getByTestId("tablet-tray").contains(view.getByTestId("the-tray"))).toBe(true);
});

test("the menu holds the menu bar's commands", async () => {
  const w = win();
  const view = render(<TabletShell win={w} />);
  fireEvent.click(view.getByTestId("tablet-menu"));
  fireEvent.click(await view.findByTestId("menu-item-shell.menu.file"));
  fireEvent.click(await view.findByTestId("menu-item-save"));
  expect(w.log).toEqual(["save"]);
});

// Continue from another device: read as the menu opens.
test("the menu has Continue from ▸ another device's pages", async () => {
  const w = win({ continueFrom: async () => [{ id: "continue.dev-1", label: "Mac", enabled: true, run: () => {} }] });
  const view = render(<TabletShell win={w} />);
  fireEvent.click(view.getByTestId("tablet-menu"));
  fireEvent.click(await view.findByTestId("menu-item-shell.menu.continue"));
  expect((await view.findByTestId("menu-item-continue.dev-1")).textContent).toContain("Mac");
});

// The contract: every contribution has a place in each shell, or is a
// named gap. The gaps shrink to none as the tablet shell is built.
test("the tablet shell's gaps are the known ones", () => {
  const gaps = SHELL_CONTRIBUTIONS.filter((c) => SHELL_PLACES.tablet[c] === null);
  expect(gaps).toEqual([]);
  expect(SHELL_CONTRIBUTIONS.filter((c) => SHELL_PLACES.desktop[c] === null)).toEqual([]);
});
