import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Stream, Thread, ThreadState } from "../api.js";
import type { HostProject } from "../host/hostApi.js";
import { requestNewThread } from "../new-thread-bus.js";
import type { ShellWindow } from "./shellWindow.js";
import { TabletShell } from "./TabletShell.js";

afterEach(cleanup);

const MAIN = { id: "str1", kind: "primary", title: "Main", worktree_path: null } as unknown as Stream;
const SIDE = { id: "str2", kind: "worktree", title: "Side", worktree_path: null } as unknown as Stream;
const WRITER = { id: "thr1", stream_id: "str1", title: "Writer", mark: null } as unknown as Thread;
const STATES: Record<string, ThreadState> = { str1: { selectedThreadId: "thr1", threads: [WRITER] }, str2: { selectedThreadId: null, threads: [] } };

function win(log: string[]): ShellWindow {
  const say = (s: string) => () => void log.push(s);
  return {
    streams: [MAIN, SIDE],
    stream: MAIN,
    vcsEnabled: true,
    threadStates: STATES,
    selectedThread: WRITER,
    streamStatuses: {},
    agentStatuses: { thr1: "working" },
    agentQuestions: {},
    switchStream: (id) => void log.push(`switch ${id}`),
    selectThread: (s, t) => void log.push(`select ${s} ${t}`),
    createThread: async (s, t) => (log.push(`create ${s} ${t}`), true),
    renameStream: (id, t) => void log.push(`rename stream ${id} ${t}`),
    renameThread: (id, t) => void log.push(`rename thread ${id} ${t}`),
    markThread: say("mark"),
    closeThread: say("close thread"),
    openStreamSettings: say("stream settings"),
    openThreadSettings: say("thread settings"),
    openNewStream: say("new stream"),
    tabs: [],
    activeTabId: null,
    besideTabId: null,
    setBeside: say("beside"),
    activateTab: say("activate"),
    closeTab: say("close"),
    reorderTabs: say("order"),
    continueFrom: async () => [],
    aside: null,
    gitDialogs: null,
    statusBar: null,
    openPage: say("open"),
    openSearch: say("search"),
    menu: [],
    tray: null,
  };
}

test("the context line opens the switcher; a thread's tap selects it and closes it", async () => {
  const log: string[] = [];
  const view = render(<TabletShell win={win(log)} />);
  fireEvent.click(view.getByTestId("tablet-context"));
  fireEvent.click(await view.findByTestId("switcher-thread-thr1"));
  expect(log).toEqual(["select str1 thr1"]);
  await waitFor(() => expect(view.queryByTestId("switcher-thread-thr1") === null).toBe(true));
});

test("a stream's tap switches to it", async () => {
  const log: string[] = [];
  const view = render(<TabletShell win={win(log)} />);
  fireEvent.click(view.getByTestId("tablet-context"));
  fireEvent.click(await view.findByTestId("switcher-stream-str2"));
  expect(log).toEqual(["switch str2"]);
});

test("New Thread from anywhere opens the switcher with the inline title field", async () => {
  const log: string[] = [];
  const view = render(<TabletShell win={win(log)} />);
  act(() => requestNewThread("str2"));
  const field = await view.findByTestId("switcher-new-thread-input-str2");
  fireEvent.change(field, { target: { value: "Plan" } });
  fireEvent.submit(field.closest("form")!);
  await waitFor(() => expect(log).toEqual(["create str2 Plan"]));
  await waitFor(() => expect(view.queryByTestId("switcher-new-thread-input-str2") === null).toBe(true));
});

// A blur with text keeps the field (a tap on the panel's × makes no thread);
// Escape drops it and leaves the switcher open.
test("the new thread field is made only by Enter; Escape closes only the field", async () => {
  const log: string[] = [];
  const view = render(<TabletShell win={win(log)} />);
  act(() => requestNewThread("str2"));
  const field = await view.findByTestId("switcher-new-thread-input-str2");
  fireEvent.change(field, { target: { value: "Plan" } });
  fireEvent.blur(field);
  expect(view.getByTestId("switcher-new-thread-input-str2")).not.toBeNull();
  fireEvent.keyDown(field, { key: "Escape" });
  expect(view.queryByTestId("switcher-new-thread-input-str2")).toBeNull();
  expect(view.getByTestId("tablet-switcher")).not.toBeNull();
  expect(log).toEqual([]);
});

test("Rename from a thread's menu is an inline field", async () => {
  const log: string[] = [];
  const view = render(<TabletShell win={win(log)} />);
  fireEvent.click(view.getByTestId("tablet-context"));
  fireEvent.contextMenu(await view.findByTestId("switcher-thread-thr1"), { clientX: 5, clientY: 5 });
  fireEvent.click(await view.findByTestId("menu-item-thread.rename"));
  const field = await view.findByTestId("switcher-rename-input");
  fireEvent.change(field, { target: { value: "Editor" } });
  fireEvent.keyDown(field, { key: "Enter" });
  await waitFor(() => expect(log).toEqual(["rename thread thr1 Editor"]));
});

test("on a host's page the switcher lists its projects; another one opens with a hold", async () => {
  const log: string[] = [];
  const projects: HostProject[] = [
    { id: "p1", name: "oxplow", state: "running", holds: 1, held_by_you: true },
    { id: "p2", name: "notes", state: "stopped", holds: 0, held_by_you: false },
  ];
  const host = {
    origin: "https://mac.local:7400",
    token: "t",
    current: "p1",
    listProjects: async () => projects,
    setHold: async (_h: string, _t: string, id: string, held: boolean) => void log.push(`hold ${id} ${held}`),
    go: (path: string) => void log.push(`go ${path}`),
  };
  const view = render(<TabletShell win={win(log)} host={host} />);
  fireEvent.click(view.getByTestId("tablet-context"));
  expect((await view.findByTestId("switcher-project-p1")).getAttribute("aria-current")).toBe("true");
  fireEvent.click(view.getByTestId("switcher-project-p2"));
  await waitFor(() => expect(log).toEqual(["hold p2 true", "go /p/p2/"]));
});
