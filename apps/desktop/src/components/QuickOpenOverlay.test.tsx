import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { Stream } from "../api.js";

// The launcher's pages are the extensions' page files: one extension has
// Tasks.
const realApi = await import("../api.js");
const pristineApi = { ...realApi };
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [
    {
      name: "oxplow-foundation",
      enabled: true,
      ui: { slots: [], commands: [], decorators: [] },
      namespace: "oxplow",
      panels: [],
      pageFiles: [
        {
          id: "oxplow-foundation/tasks",
          extension: "oxplow-foundation",
          slug: "tasks",
          title: "Tasks",
          category: "Work",
          keywords: null,
          hidden: false,
          for: [],
          params: [],
          badge: null,
          view: { id: "v", kind: "markdown", text: "" },
        },
      ],
    },
  ],
}));
const { QuickOpenOverlay } = await import("./QuickOpenOverlay.js");

// `mock.module` is process-wide: the files after this one get the real module.
afterAll(() => {
  mock.module("../api.js", () => pristineApi);
});

afterEach(cleanup);

const stream = { id: "str1", title: "oxplow" } as unknown as Stream;


function renderOverlay() {
  return render(
    <QuickOpenOverlay
      open
      stream={stream}
      threadId="thr1"
      selectedFilePath={null}
      offers={[]}
      onClose={() => {}}
      onOpenFile={() => {}}
      onOpenPage={() => {}}
      onOpenSearchHit={() => {}}
    />,
  );
}

// The launcher tree still renders after the Recent-section wiring: static
// category headers show, and with no backend the recent-visit fetch rejects
// → the Recent section stays absent (rather than crashing the overlay).
test("renders the static launcher tree and omits Recent when there are no visits", async () => {
  const { findByTestId, queryByTestId } = renderOverlay();
  // The page list waits for the extensions (here none: no backend).
  expect(await findByTestId("launcher-category-Work")).toBeTruthy();
  expect(queryByTestId("launcher-category-Recent")).toBeNull();
});

// Opened before the window's stream has loaded, the launcher shows — and
// takes the focus — once it has; a query typed then survives the stream
// being restated (only opening starts a fresh one).
test("the launcher focuses when its stream arrives and keeps what was typed", () => {
  const props = {
    open: true,
    threadId: "thr1",
    selectedFilePath: null,
    offers: [],
    onClose: () => {},
    onOpenFile: () => {},
    onOpenPage: () => {},
    onOpenSearchHit: () => {},
  };
  const view = render(<QuickOpenOverlay {...props} stream={null} />);
  expect(view.queryByPlaceholderText(/Search everything/)).toBeNull();
  view.rerender(<QuickOpenOverlay {...props} stream={stream} />);
  const input = view.getByPlaceholderText(/Search everything/) as HTMLInputElement;
  expect(document.activeElement).toBe(input);
  fireEvent.change(input, { target: { value: "Settings" } });
  view.rerender(<QuickOpenOverlay {...props} stream={{ ...stream } as Stream} />);
  expect(input.value).toBe("Settings");
});

// After a reload, the window settles under an open launcher: a restored tab
// or terminal mounting takes focus, and the keys that follow went to neither
// (tsk1055). The launcher keeps focus while it is open.
test("something else taking focus doesn't cost the launcher its keystrokes", async () => {
  const view = renderOverlay();
  const input = view.getByPlaceholderText(/Search everything/) as HTMLInputElement;
  expect(document.activeElement).toBe(input);
  const other = document.createElement("textarea");
  document.body.appendChild(other);
  other.focus();
  expect(document.activeElement).toBe(other);
  // It takes focus back on the next frame. Wait for that frame, not a
  // poll: waitFor's interval stalled for seconds under the full suite.
  await new Promise<void>((resolve) => requestAnimationFrame(() => setTimeout(resolve, 0)));
  expect(document.activeElement).toBe(input);
  other.remove();
});

test("focus moving inside the launcher stays there", async () => {
  const view = renderOverlay();
  const input = view.getByPlaceholderText(/Search everything/) as HTMLInputElement;
  const inside = document.createElement("button");
  input.parentElement!.appendChild(inside);
  inside.focus();
  await new Promise((r) => setTimeout(r, 50));
  expect(document.activeElement).toBe(inside);
});

test("a closed launcher gives focus back to whatever has it", async () => {
  const props = {
    stream,
    threadId: "thr1",
    selectedFilePath: null,
    offers: [],
    onClose: () => {},
    onOpenFile: () => {},
    onOpenPage: () => {},
    onOpenSearchHit: () => {},
  };
  const view = render(<QuickOpenOverlay {...props} open />);
  const other = document.createElement("textarea");
  document.body.appendChild(other);
  view.rerender(<QuickOpenOverlay {...props} open={false} />);
  other.focus();
  await new Promise((r) => setTimeout(r, 50));
  expect(document.activeElement).toBe(other);
  other.remove();
});

// The launcher follows the one list contract (useListNav): ↑/↓ move and stop
// at the ends, Enter takes the row under the cursor, Escape closes, and the
// input names the cursor's row.
test("↑/↓ and Enter work from the input, and the input names the row under the cursor", async () => {
  const opened: string[] = [];
  let closed = 0;
  const view = render(
    <QuickOpenOverlay
      open
      stream={stream}
      threadId="thr1"
      selectedFilePath={null}
      offers={[]}
      onClose={() => closed++}
      onOpenFile={() => {}}
      onOpenPage={(ref) => opened.push(ref.id)}
      onOpenSearchHit={() => {}}
    />,
  );
  const input = view.getByPlaceholderText(/Search everything/);
  await view.findByTestId("launcher-category-Work");
  // Enter on the collapsed section opens it; the page row appears.
  fireEvent.keyDown(input, { key: "Enter" });
  expect(view.getByTestId("launcher-page-oxplow:oxplow/page?page=tasks")).toBeTruthy();
  fireEvent.keyDown(input, { key: "ArrowDown" });
  fireEvent.keyDown(input, { key: "ArrowDown" });
  const active = input.getAttribute("aria-activedescendant");
  expect(document.getElementById(active!)?.getAttribute("data-testid")).toBe("launcher-page-oxplow:oxplow/page?page=tasks");
  fireEvent.keyDown(input, { key: "Enter" });
  expect(opened).toEqual(["oxplow:oxplow/page?page=tasks"]);
  // Taking a page closes the launcher; so does Escape.
  const afterPick = closed;
  fireEvent.keyDown(input, { key: "Escape" });
  expect(closed).toBe(afterPick + 1);
});

// Pressing a row (a section header that opens, say) leaves the typing in
// the input, so the keys that follow — Escape, ↑/↓ — still work.
test("pressing a row keeps the input focused, so Escape still closes", async () => {
  let closed = 0;
  const view = render(
    <QuickOpenOverlay
      open
      stream={stream}
      threadId="thr1"
      selectedFilePath={null}
      offers={[]}
      onClose={() => closed++}
      onOpenFile={() => {}}
      onOpenPage={() => {}}
      onOpenSearchHit={() => {}}
    />,
  );
  const input = view.getByPlaceholderText(/Search everything/);
  input.focus();
  const header = await view.findByTestId("launcher-category-Work");
  // A press whose default isn't prevented moves focus to the button.
  const pressed = fireEvent.mouseDown(header);
  expect(pressed).toBe(false);
  fireEvent.click(header);
  expect(document.activeElement).toBe(input);
  fireEvent.keyDown(input, { key: "Escape" });
  expect(closed).toBe(1);
});

// Opened with a query (Find Usages from a file's menu), the box starts on it.
test("a launcher opened with a query starts on it", () => {
  const view = render(
    <QuickOpenOverlay
      open
      initialQuery="needle"
      stream={stream}
      threadId="thr1"
      selectedFilePath={null}
      offers={[]}
      onClose={() => {}}
      onOpenFile={() => {}}
      onOpenPage={() => {}}
      onOpenSearchHit={() => {}}
    />,
  );
  expect((view.container.querySelector("input") as HTMLInputElement).value).toBe("needle");
});
