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
    stream: { id: "str1", title: "Main" } as unknown as Stream,
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
    tabs: [tab("session", { pinned: true, closable: false, agentStatus: "working" }), tab("a"), tab("b")],
    activeTabId: "a",
    activateTab: (id) => log.push(`activate ${id}`),
    closeTab: (id) => log.push(`close ${id}`),
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
  expect(view.getByTestId("tablet-context").textContent).toContain("Main");
  expect(view.getByTestId("tablet-context").textContent).toContain("Writer");
  expect(view.getByTestId("center-tab-body-a").style.display).toBe("flex");
  expect(view.queryByTestId("center-tab-body-b")).toBeNull();
});

test("a tap shows a tab, its × closes it; a pinned session has no ×", () => {
  const w = win();
  const view = render(<TabletShell win={w} />);
  fireEvent.click(view.getByTestId("tablet-tab-b"));
  fireEvent.click(view.getByTestId("tablet-tab-close-a"));
  expect(view.queryByTestId("tablet-tab-close-session")).toBeNull();
  expect(w.log).toEqual(["activate b", "close a"]);
});

test("a tab's menu is the window's tab menu", () => {
  const w = win();
  const view = render(<TabletShell win={w} />);
  fireEvent.contextMenu(view.getByTestId("tablet-tab-a"), { clientX: 10, clientY: 10 });
  fireEvent.click(view.getByTestId("menu-item-tab.close-others"));
  expect(w.log).toEqual(["close b"]);
});

test("search and the tray are drawn", () => {
  const w = win();
  const view = render(<TabletShell win={w} />);
  fireEvent.click(view.getByTestId("tablet-search"));
  expect(w.log).toEqual(["search"]);
  expect(view.getByTestId("tablet-tray").contains(view.getByTestId("the-tray"))).toBe(true);
});

test("the menu holds the menu bar's commands", async () => {
  const w = win();
  const view = render(<TabletShell win={w} />);
  fireEvent.click(view.getByTestId("tablet-menu"));
  fireEvent.click(view.getByTestId("menu-item-shell.menu.file"));
  fireEvent.click(await view.findByTestId("menu-item-save"));
  expect(w.log).toEqual(["save"]);
});

// The contract: every contribution has a place in each shell, or is a
// named gap. The gaps shrink to none as the tablet shell is built.
test("the tablet shell's gaps are the known ones", () => {
  const gaps = SHELL_CONTRIBUTIONS.filter((c) => SHELL_PLACES.tablet[c] === null);
  expect(gaps).toEqual(["panels"]);
  expect(SHELL_CONTRIBUTIONS.filter((c) => SHELL_PLACES.desktop[c] === null)).toEqual([]);
});
