import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Stream, Thread, ThreadState } from "../api.js";
import { requestNavigatorMenu, requestNavigatorOpen } from "../navigator-bus.js";
import { Navigator } from "./Navigator.js";

afterEach(cleanup);

const NOOP_ASYNC = async () => {};

/** The overlay's pointer-leave grace (180ms) plus slack. */
const AFTER_GRACE = 260;
const settle = () => new Promise((r) => setTimeout(r, AFTER_GRACE));

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

function renderNavigator(opts?: {
  onPromoteThread?: (threadId: string) => void;
  onSwitchStream?: (streamId: string) => void;
  onSelectThread?: (streamId: string, threadId: string) => void;
  onRenameThread?: (threadId: string, title: string) => void;
}) {
  return render(
    <Navigator
      streams={[STREAM]}
      currentStreamId="str1"
      threadStates={THREAD_STATES}
      streamStatuses={{}}
      agentStatuses={{}}
      enabledAgents={["claude"]}
      onSwitchStream={opts?.onSwitchStream ?? NOOP_ASYNC}
      onSelectThread={opts?.onSelectThread ?? NOOP_ASYNC}
      onCreateThread={NOOP_ASYNC}
      onPromoteThread={opts?.onPromoteThread}
      onRenameThread={opts?.onRenameThread}
      vcsEnabled
    />,
  );
}

/**
 * The overlay opens by CLICKING the strip's bottom-pinned chevron —
 * hover no longer opens it (tsk269).
 */
function openOverlay(getByTestId: (id: string) => HTMLElement) {
  fireEvent.click(getByTestId("navigator-expand"));
  return getByTestId("navigator-overlay");
}

/**
 * Give the overlay panel a real rect so the geometric pointer-leave test
 * has something to compare against — happy-dom reports all-zero rects.
 */
function stubPanelRect(panel: HTMLElement, right = 280, bottom = 800) {
  Object.defineProperty(panel, "getBoundingClientRect", {
    configurable: true,
    value: () => ({
      left: 0,
      top: 0,
      right,
      bottom,
      width: right,
      height: bottom,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }),
  });
}

// --- Hover no longer expands (tsk269) ------------------------------------

test("hovering the strip does not open the overlay", () => {
  const { getByTestId, queryByTestId } = renderNavigator();

  // The whole point of tsk269: a pointer that merely drifts left while
  // aiming at the rail HUD must not throw a 280px panel over the rail.
  fireEvent.mouseEnter(getByTestId("navigator-strip").parentElement as HTMLElement);
  fireEvent.mouseEnter(getByTestId("navigator-strip"));

  expect(queryByTestId("navigator-overlay") === null).toBe(true);
});

test("strip glyphs carry their full title as a tooltip", () => {
  const { getByTestId } = renderNavigator();

  // Hover's only job now is answering "what is this glyph?" — so the
  // title has to be on the row itself.
  expect(getByTestId("navigator-strip-stream-str1").title).toBe("Main");
  expect(getByTestId("navigator-strip-thread-thr1").title).toBe("Writer");
  expect(getByTestId("navigator-strip-thread-thr2").title).toBe("Research");
});

test("clicking a stream glyph in the strip switches to that stream and expands the panel", () => {
  const switched: string[] = [];
  const selected: string[] = [];
  const { getByTestId, queryByTestId } = renderNavigator({
    onSwitchStream: (id) => switched.push(id),
    onSelectThread: (_s, t) => selected.push(t),
  });

  fireEvent.click(getByTestId("navigator-strip-stream-str1"));

  expect(switched).toEqual(["str1"]);
  // Dispatching onSelectThread alongside it races the thread-state
  // writes and can leave the old thread selected — switch only.
  expect(selected).toEqual([]);
  // A stream glyph heads its threads: it opens the panel to show them.
  expect(queryByTestId("navigator-overlay") !== null).toBe(true);
});

test("clicking a thread glyph in the strip selects that thread", () => {
  const selected: string[] = [];
  const { getByTestId, queryByTestId } = renderNavigator({ onSelectThread: (_s, t) => selected.push(t) });

  fireEvent.click(getByTestId("navigator-strip-thread-thr2"));

  expect(selected).toEqual(["thr2"]);
  expect(queryByTestId("navigator-overlay") === null).toBe(true);
});

test("clicking the selected thread's glyph expands the panel", () => {
  const selected: string[] = [];
  const { getByTestId, queryByTestId } = renderNavigator({ onSelectThread: (_s, t) => selected.push(t) });

  // Selecting it again would do nothing, so the click opens the panel —
  // as a click in the strip's empty space does.
  fireEvent.click(getByTestId("navigator-strip-thread-thr1"));

  expect(selected).toEqual([]);
  expect(queryByTestId("navigator-overlay") !== null).toBe(true);
});

// --- Expanding ------------------------------------------------------------

test("the bottom-pinned chevron expands the panel", () => {
  const { getByTestId, queryByTestId } = renderNavigator();

  expect(queryByTestId("navigator-overlay") === null).toBe(true);
  fireEvent.click(getByTestId("navigator-expand"));
  expect(queryByTestId("navigator-overlay") !== null).toBe(true);
});

test("the expand and collapse toggles show the open/close-sidebar icons", () => {
  const { getByTestId } = renderNavigator();

  // A lone "›" didn't read as "show the streams panel"; the standard
  // sidebar icons do.
  expect(getByTestId("navigator-expand").querySelector("svg.lucide-panel-left-open")).not.toBeNull();
  fireEvent.click(getByTestId("navigator-expand"));
  expect(getByTestId("navigator-collapse").querySelector("svg.lucide-panel-left-close")).not.toBeNull();
});

test("a request from elsewhere (the title bar's stream name) expands the panel", () => {
  const { queryByTestId } = renderNavigator();
  act(() => requestNavigatorOpen());
  expect(queryByTestId("navigator-overlay") !== null).toBe(true);
});

test("clicking empty space in the strip expands the panel", () => {
  const { getByTestId, queryByTestId } = renderNavigator();

  // The scroll container's own background — i.e. below the last stream
  // panel. A bonus route in; the chevron is the discoverable one.
  const empty = getByTestId("navigator-strip-empty");
  fireEvent.click(empty, { target: empty });
  expect(queryByTestId("navigator-overlay") !== null).toBe(true);
});

// --- Collapsing -----------------------------------------------------------

test("a pointerdown outside the overlay collapses it so rail clicks aren't intercepted", () => {
  const { getByTestId, queryByTestId } = renderNavigator();

  openOverlay(getByTestId);
  expect(queryByTestId("navigator-overlay") !== null).toBe(true);

  // A press anywhere outside the overlay (the rail / center / tab bar)
  // must collapse it immediately (tsk131) — the very next click then
  // lands on the rail instead of being swallowed by the overlay.
  fireEvent.pointerDown(document.body);
  expect(queryByTestId("navigator-overlay") === null).toBe(true);
});

test("Escape collapses the expanded overlay", () => {
  const { getByTestId, queryByTestId } = renderNavigator();

  openOverlay(getByTestId);
  expect(queryByTestId("navigator-overlay") !== null).toBe(true);

  fireEvent.keyDown(document, { key: "Escape" });
  expect(queryByTestId("navigator-overlay") === null).toBe(true);
});

test("clicking an empty area inside the panel closes it", () => {
  const { getByTestId, queryByTestId } = renderNavigator();

  const panel = openOverlay(getByTestId);
  fireEvent.click(panel, { target: panel });

  expect(queryByTestId("navigator-overlay") === null).toBe(true);
});

test("clicking a control inside the panel does NOT close it", () => {
  const { getByTestId, queryByTestId } = renderNavigator();

  openOverlay(getByTestId);
  // Open the inline new-thread form from the stream row's menu, then
  // interact with its input — a control click must never dismiss.
  fireEvent.contextMenu(getByTestId("navigator-stream-row-str1"));
  fireEvent.click(getByTestId("menu-item-stream.add-thread"));

  const input = getByTestId("navigator-new-thread-input");
  fireEvent.click(input);

  expect(queryByTestId("navigator-overlay") !== null).toBe(true);
  expect(queryByTestId("navigator-new-thread-input") !== null).toBe(true);
});

test("the panel closes when the pointer moves outside its bounds", async () => {
  const { getByTestId, queryByTestId } = renderNavigator();

  const panel = openOverlay(getByTestId);
  stubPanelRect(panel);

  // Geometric, not `mouseleave`: the panel covers the rail HUD, and
  // because it's inside the wrapper's subtree the wrapper's mouseleave
  // never fires while the pointer sits over the covered region.
  fireEvent.pointerMove(document, { clientX: 40, clientY: 300 });
  fireEvent.pointerMove(document, { clientX: 400, clientY: 300 });
  await waitFor(() => expect(queryByTestId("navigator-overlay") === null).toBe(true));
});

test("opened from outside it (the title bar), the panel stays open until the pointer has been in it", async () => {
  const { getByTestId, queryByTestId } = renderNavigator();
  act(() => requestNavigatorOpen());
  stubPanelRect(getByTestId("navigator-overlay"));

  // The pointer is over the title bar, above the panel, and drifts there.
  fireEvent.pointerMove(document, { clientX: 120, clientY: -10 });
  fireEvent.pointerMove(document, { clientX: 140, clientY: -12 });
  await settle();
  expect(queryByTestId("navigator-overlay") !== null).toBe(true);

  // Once it has been in the panel, leaving it closes it as usual.
  fireEvent.pointerMove(document, { clientX: 100, clientY: 200 });
  fireEvent.pointerMove(document, { clientX: 400, clientY: 300 });
  await waitFor(() => expect(queryByTestId("navigator-overlay") === null).toBe(true));
});

test("a pointer still inside the panel keeps it open", async () => {
  const { getByTestId, queryByTestId } = renderNavigator();

  const panel = openOverlay(getByTestId);
  stubPanelRect(panel);

  fireEvent.pointerMove(document, { clientX: 120, clientY: 300 });
  await act(settle);

  expect(queryByTestId("navigator-overlay") !== null).toBe(true);
});

test("the panel stays open while a form is active even if the pointer leaves", async () => {
  const { getByTestId, queryByTestId } = renderNavigator();

  const panel = openOverlay(getByTestId);
  stubPanelRect(panel);

  fireEvent.contextMenu(getByTestId("navigator-stream-row-str1"));
  fireEvent.click(getByTestId("menu-item-stream.add-thread"));

  // Mid-new-thread the user is committed to an action inside the panel —
  // an overshoot with the mouse must not throw their typing away.
  fireEvent.pointerMove(document, { clientX: 400, clientY: 300 });
  await act(settle);

  expect(queryByTestId("navigator-overlay") !== null).toBe(true);
  expect(queryByTestId("navigator-new-thread-input") !== null).toBe(true);
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
  expect(queryByTestId("menu-item-thread.promote") === null).toBe(true);
  // The menu still renders its other actions.
  expect(queryByTestId("menu-item-thread.rename") !== null).toBe(true);
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

// --- Right-click on the strip's icons ---------------------------------------

test("right-clicking a thread icon in the strip opens its row menu, headed by its title", () => {
  const promoted: string[] = [];
  const { getByTestId, queryByTestId } = renderNavigator({ onPromoteThread: (id) => promoted.push(id) });

  fireEvent.contextMenu(getByTestId("navigator-strip-thread-thr2"));

  // The icon shows only initials, so the menu names the thread.
  expect(getByTestId("context-menu-header").textContent).toBe("Research");
  // The same items as the expanded row's menu.
  expect(getByTestId("menu-item-thread.promote")).toBeTruthy();
  expect(getByTestId("menu-item-thread.rename")).toBeTruthy();
  expect(getByTestId("menu-item-thread.settings")).toBeTruthy();
  expect(getByTestId("menu-item-thread.close")).toBeTruthy();
  // Sessions start from the thread's own page, not its menu.
  expect(queryByTestId("menu-item-thread.new-session") === null).toBe(true);
  fireEvent.click(getByTestId("menu-item-thread.promote"));
  expect(promoted).toEqual(["thr2"]);
  // Right-click opens a menu, not the panel.
  expect(queryByTestId("navigator-overlay") === null).toBe(true);
});

test("right-clicking a stream icon in the strip opens the stream's menu, headed by its title", () => {
  const { getByTestId } = renderNavigator();

  fireEvent.contextMenu(getByTestId("navigator-strip-stream-str1"));

  expect(getByTestId("context-menu-header").textContent).toBe("Main");
  expect(getByTestId("menu-item-stream.add-thread")).toBeTruthy();
  expect(getByTestId("menu-item-stream.rename")).toBeTruthy();
  expect(getByTestId("menu-item-stream.settings")).toBeTruthy();
});

test("Rename from a strip icon's menu opens the panel with the rename field", () => {
  const { getByTestId, queryByTestId } = renderNavigator({ onRenameThread: () => {} });

  fireEvent.contextMenu(getByTestId("navigator-strip-thread-thr2"));
  fireEvent.click(getByTestId("menu-item-thread.rename"));

  expect(queryByTestId("navigator-overlay") !== null).toBe(true);
  expect(getByTestId("navigator-thread-row-thr2").querySelector("input")).not.toBeNull();
});

test("the expanded rows' menus are headed by the name too", () => {
  const { getByTestId } = renderNavigator();
  openOverlay(getByTestId);
  fireEvent.contextMenu(getByTestId("navigator-thread-row-thr2"));
  expect(getByTestId("menu-item-thread.rename")).toBeTruthy();
  expect(getByTestId("context-menu-header").textContent).toBe("Research");
});

test("a menu request from elsewhere (the title bar) opens that row's menu, headed by its name", () => {
  const promoted: string[] = [];
  const { getByTestId, queryByTestId } = renderNavigator({ onPromoteThread: (id) => promoted.push(id) });

  act(() => requestNavigatorMenu({ kind: "thread", id: "thr2", x: 200, y: 15 }));
  expect(getByTestId("context-menu-header").textContent).toBe("Research");
  fireEvent.click(getByTestId("menu-item-thread.promote"));
  expect(promoted).toEqual(["thr2"]);

  act(() => requestNavigatorMenu({ kind: "stream", id: "str1", x: 120, y: 14 }));
  expect(getByTestId("context-menu-header").textContent).toBe("Main");
  expect(getByTestId("menu-item-stream.add-thread")).toBeTruthy();
  // A menu, not the panel.
  expect(queryByTestId("navigator-overlay") === null).toBe(true);
});

test("streams are inverted tiles and threads indented tabs, tied by a guide line", () => {
  const { getByTestId } = renderNavigator();
  const glyph = (rowId: string) =>
    getByTestId(rowId).querySelector("[data-glyph]") as HTMLElement;

  // Tabs: rounded on the left only, flush with the strip's right edge.
  const stream = glyph("navigator-strip-stream-str1");
  expect(stream.dataset.glyph).toBe("stream");
  expect(stream.style.background).toBe("var(--surface-stream-tile)");
  expect(stream.style.color).toBe("var(--text-on-stream-tile)");
  expect(stream.style.borderRadius).toBe("6px 0px 0px 6px");

  // A stream's threads are tabs butted up against each other: each fills
  // its row, and only the last closes the stack along its bottom.
  const writer = glyph("navigator-strip-thread-thr1");
  const queued = glyph("navigator-strip-thread-thr2");
  for (const [rowId, t] of [
    ["navigator-strip-thread-thr1", writer],
    ["navigator-strip-thread-thr2", queued],
  ] as const) {
    expect(t.dataset.glyph).toBe("thread");
    expect(t.style.borderRadius).toBe("4px 0px 0px 4px");
    expect(t.style.height).toBe(getByTestId(rowId).style.height);
  }
  expect(writer.style.borderWidth).toBe("1px 0px 0px 1px");
  expect(queued.style.borderWidth).toBe("1px 0px 1px 1px");
  expect(writer.style.borderColor).toBe("var(--accent)");
  expect(queued.style.borderColor).toBe("var(--border-strong)");

  // The guide runs from the stream through its threads, ending at the last.
  const guide = (rowId: string) =>
    getByTestId(rowId).querySelector("[data-guide]")?.getAttribute("data-guide") ?? null;
  expect(guide("navigator-strip-stream-str1")).toBe("stream");
  expect(guide("navigator-strip-thread-thr1")).toBe("mid");
  expect(guide("navigator-strip-thread-thr2")).toBe("last");

  // The panel draws the same, so its rows stay lined up with the strip's.
  openOverlay(getByTestId);
  expect(guide("navigator-stream-row-str1")).toBe("stream");
  expect(guide("navigator-thread-row-thr2")).toBe("last");
  expect(
    (getByTestId("navigator-thread-row-thr1").querySelector("[data-glyph]") as HTMLElement).dataset.glyph,
  ).toBe("thread");
});
