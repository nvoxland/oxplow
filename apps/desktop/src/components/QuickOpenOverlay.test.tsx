import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { Stream } from "../api.js";
import { QuickOpenOverlay } from "./QuickOpenOverlay.js";
import type { PageDirectoryEntry } from "./RailHud/sections.js";

afterEach(cleanup);

const stream = { id: "str1", title: "oxplow" } as unknown as Stream;

const pages: PageDirectoryEntry[] = [
  { id: "tasks", label: "Tasks", category: "Work", ref: { id: "tasks", kind: "tasks", payload: null } as PageDirectoryEntry["ref"] },
];

function renderOverlay() {
  return render(
    <QuickOpenOverlay
      open
      stream={stream}
      threadId="thr1"
      selectedFilePath={null}
      pages={pages}
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
test("renders the static launcher tree and omits Recent when there are no visits", () => {
  const { getByTestId, queryByTestId } = renderOverlay();
  expect(getByTestId("launcher-category-Work")).toBeTruthy();
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
    pages,
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
