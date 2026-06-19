import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { Stream, Thread, ThreadState } from "../api.js";
import { Navigator } from "./Navigator.js";

afterEach(cleanup);

const NOOP_ASYNC = async () => {};

const STREAM = {
  id: "str1",
  kind: "primary",
  title: "Main",
  branch: "main",
} as unknown as Stream;

const WRITER = {
  id: "thr1",
  stream_id: "str1",
  title: "Writer",
  status: "active",
} as unknown as Thread;

// A second thread that is NOT the active writer — i.e. queued / read-only
// (the write guard allows one writer per stream).
const QUEUED = {
  id: "thr2",
  stream_id: "str1",
  title: "Research",
  status: "queued",
} as unknown as Thread;

const THREAD_STATES: Record<string, ThreadState> = {
  str1: { selectedThreadId: "thr1", activeThreadId: "thr1", threads: [WRITER, QUEUED] },
};

function renderNavigator(opts?: { onPromoteThread?: (threadId: string) => void }) {
  return render(
    <Navigator
      streams={[STREAM]}
      currentStreamId="str1"
      threadStates={THREAD_STATES}
      streamStatuses={{}}
      agentStatuses={{}}
      enabledAgents={["claude"]}
      onSwitchStream={NOOP_ASYNC}
      onSelectThread={NOOP_ASYNC}
      onCreateThread={NOOP_ASYNC}
      onPromoteThread={opts?.onPromoteThread}
      gitEnabled
    />,
  );
}

/** The overlay opens on mouse-enter of the navigator wrapper. */
function openOverlay(getByTestId: (id: string) => HTMLElement) {
  const wrapper = getByTestId("navigator-strip").parentElement as HTMLElement;
  fireEvent.mouseEnter(wrapper);
  return wrapper;
}

test("a pointerdown outside the overlay collapses it so rail clicks aren't intercepted", () => {
  const { getByTestId, queryByTestId } = renderNavigator();

  openOverlay(getByTestId);
  // Overlay is expanded — it would otherwise cover the rail to its right.
  expect(queryByTestId("navigator-overlay")).not.toBeNull();

  // A press anywhere outside the overlay (the rail / center / tab bar)
  // must collapse it immediately (tsk131) — the very next click then
  // lands on the rail instead of being swallowed by the overlay.
  fireEvent.pointerDown(document.body);
  expect(queryByTestId("navigator-overlay")).toBeNull();
});

test("Escape collapses the expanded overlay", () => {
  const { getByTestId, queryByTestId } = renderNavigator();

  openOverlay(getByTestId);
  expect(queryByTestId("navigator-overlay")).not.toBeNull();

  fireEvent.keyDown(document, { key: "Escape" });
  expect(queryByTestId("navigator-overlay")).toBeNull();
});

test("a press inside the overlay keeps it open (rows stay interactive)", () => {
  const { getByTestId, queryByTestId } = renderNavigator();

  openOverlay(getByTestId);
  const panel = getByTestId("navigator-overlay");
  expect(panel).not.toBeNull();

  fireEvent.pointerDown(panel);
  // Pressing within the overlay (e.g. on a stream/thread row)
  // must NOT dismiss it — only outward interaction does.
  expect(queryByTestId("navigator-overlay")).not.toBeNull();
});

// --- "Make writer" promote action (tsk132) -------------------------------

/** Open a thread row's right-click menu inside the expanded overlay. */
function openThreadMenu(
  getByTestId: (id: string) => HTMLElement,
  threadId: string,
) {
  openOverlay(getByTestId);
  fireEvent.contextMenu(getByTestId(`navigator-thread-row-${threadId}`));
}

test("a read-only (non-writer) thread's menu leads with an enabled 'Make writer'", () => {
  const { getByTestId } = renderNavigator({ onPromoteThread: () => {} });
  openThreadMenu(getByTestId, "thr2");

  const promote = getByTestId("menu-item-thread.promote");
  expect(promote.textContent).toContain("Make writer");
  expect((promote as HTMLButtonElement).disabled).toBe(false);

  // It is the headline action — ahead of Rename — so a user whose new
  // thread is "edits blocked" finds the way out first.
  const rename = getByTestId("menu-item-thread.rename");
  expect(promote.compareDocumentPosition(rename) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

test("the active writer's menu does NOT offer 'Make writer'", () => {
  const { getByTestId, queryByTestId } = renderNavigator();
  openThreadMenu(getByTestId, "thr1");

  // The writer is already writable; only queued/read-only threads can be
  // promoted, so the action is absent rather than shown-but-disabled.
  expect(queryByTestId("menu-item-thread.promote")).toBeNull();
  // The menu still renders its other actions.
  expect(queryByTestId("menu-item-thread.rename")).not.toBeNull();
});

test("clicking 'Make writer' promotes that thread via the IPC handler", () => {
  const promoted: string[] = [];
  const { getByTestId } = renderNavigator({
    onPromoteThread: (id) => promoted.push(id),
  });
  openThreadMenu(getByTestId, "thr2");

  fireEvent.click(getByTestId("menu-item-thread.promote"));
  expect(promoted).toEqual(["thr2"]);
});
