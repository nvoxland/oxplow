import { afterEach, expect, test } from "bun:test";
import { act, cleanup, render, renderHook } from "@testing-library/react";

import type { Stream, Thread, ThreadState } from "../api.js";
import { requestNavigatorMenu } from "../navigator-bus.js";
import { requestNewThread } from "../new-thread-bus.js";
import { useStreamThreadMenus, type StreamThreadMenusInput } from "./useStreamThreadMenus.js";

// The stream and thread menus and their flows belong to the window, not to
// the Navigator: a shell without one (the tablet's) gets New Thread,
// Rename and Remove through the same hook.

afterEach(cleanup);

const MAIN = { id: "str1", kind: "primary", title: "Main" } as unknown as Stream;
const SIDE = { id: "str2", kind: "worktree", title: "Side", worktree_path: null } as unknown as Stream;
const THREAD = { id: "thr1", stream_id: "str1", title: "Writer", mark: null } as unknown as Thread;
const STATES: Record<string, ThreadState> = { str1: { selectedThreadId: "thr1", threads: [THREAD] } };

function input(over: Partial<StreamThreadMenusInput> = {}): StreamThreadMenusInput & { reveals: number[] } {
  const reveals: number[] = [];
  return {
    streams: [MAIN, SIDE],
    threadStates: STATES,
    streamStatuses: {},
    currentStreamId: "str1",
    onRenameStream: () => {},
    reveal: () => reveals.push(1),
    reveals,
    ...over,
  };
}

test("a New Thread request with no Navigator mounted starts the flow and reveals it", () => {
  const i = input();
  const { result } = renderHook(() => useStreamThreadMenus(i));
  act(() => requestNewThread("str2"));
  expect(result.current.pendingNewThreadFor).toBe("str2");
  expect(i.reveals.length).toBe(1);
});

test("a stream's menu: Add thread and Rename start their flows; Remove only off the primary, not while working", () => {
  const i = input({ streamStatuses: { str2: "working" } });
  const { result } = renderHook(() => useStreamThreadMenus(i));
  const main = result.current.streamMenu(MAIN);
  expect(main.some((m) => m.id === "stream.remove")).toBe(false);
  act(() => void main.find((m) => m.id === "stream.rename")!.run!());
  expect(result.current.renaming).toEqual({ kind: "stream", id: "str1" });
  const side = result.current.streamMenu(SIDE);
  expect(side.find((m) => m.id === "stream.remove")!.enabled).toBe(false);
  act(() => void side.find((m) => m.id === "stream.add-thread")!.run!());
  expect(result.current.pendingNewThreadFor).toBe("str2");
});

test("Remove… asks first: the removal holds the stream until confirmed or cancelled", () => {
  const i = input();
  const { result } = renderHook(() => useStreamThreadMenus(i));
  act(() => void result.current.streamMenu(SIDE).find((m) => m.id === "stream.remove")!.run!());
  expect(result.current.removal.stream?.id).toBe("str2");
  act(() => result.current.removal.cancel());
  expect(result.current.removal.stream).toBeNull();
});

test("a menu asked for from outside (the title bar) opens, headed with the stream's name", async () => {
  const i = input();
  function Host() {
    return <>{useStreamThreadMenus(i).requestedMenu}</>;
  }
  const view = render(<Host />);
  act(() => requestNavigatorMenu({ kind: "stream", id: "str2", x: 10, y: 10 }));
  expect((await view.findByTestId("context-menu-header")).textContent).toBe("Side");
});
