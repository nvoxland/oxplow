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

const THREAD = {
  id: "thr1",
  stream_id: "str1",
  title: "Writer",
  status: "active",
} as unknown as Thread;

const THREAD_STATES: Record<string, ThreadState> = {
  str1: { selectedThreadId: "thr1", activeThreadId: "thr1", threads: [THREAD] },
};

function renderNavigator() {
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
  // Pressing within the overlay (e.g. on a stream/thread row or kebab)
  // must NOT dismiss it — only outward interaction does.
  expect(queryByTestId("navigator-overlay")).not.toBeNull();
});
