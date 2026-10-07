import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { TerminalTabStrip } from "./TerminalTabStrip.js";
import type { TerminalTab } from "./terminalTabs.js";

afterEach(cleanup);

/** Past the panel's pointer-leave grace (180ms). Only for asserting a close did NOT
 *  happen — a close that should happen is awaited with `waitFor`, never a sleep. */
const settle = () => new Promise((r) => setTimeout(r, 260));

const TABS: TerminalTab[] = [
  { id: "default", title: "Terminal 1" },
  { id: "t2", title: "Build Watch" },
];

const NOOP = () => {};

function renderStrip(opts?: {
  onActivate?(id: string): void;
  onRename?(id: string, title: string): void;
  onClose?(id: string): void;
  tabs?: TerminalTab[];
}) {
  return render(
    <TerminalTabStrip
      tabs={opts?.tabs ?? TABS}
      activeId="default"
      onActivate={opts?.onActivate ?? NOOP}
      onNew={NOOP}
      onClose={opts?.onClose ?? NOOP}
      onRename={opts?.onRename ?? NOOP}
    />,
  );
}

/** Expanding is a CLICK on the bottom-pinned chevron — never hover. */
function openOverlay(getByTestId: (id: string) => HTMLElement) {
  fireEvent.click(getByTestId("terminal-tab-expand"));
  return getByTestId("terminal-tab-overlay");
}

function stubPanelRect(panel: HTMLElement, right = 272, bottom = 800) {
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

test("hovering the strip does not open the overlay", () => {
  const { getByTestId, queryByTestId } = renderStrip();

  fireEvent.mouseEnter(getByTestId("terminal-tab-strip").parentElement as HTMLElement);
  fireEvent.mouseEnter(getByTestId("terminal-tab-strip"));

  expect(queryByTestId("terminal-tab-overlay") === null).toBe(true);
});

test("glyphs carry their full title as a tooltip and activate on click", () => {
  const activated: string[] = [];
  const { getByTestId } = renderStrip({ onActivate: (id) => activated.push(id) });

  expect(getByTestId("terminal-tab-t2").title).toBe("Build Watch");
  fireEvent.click(getByTestId("terminal-tab-t2"));

  expect(activated).toEqual(["t2"]);
});

test("the bottom-pinned chevron expands the panel", () => {
  const { getByTestId, queryByTestId } = renderStrip();

  expect(queryByTestId("terminal-tab-overlay") === null).toBe(true);
  fireEvent.click(getByTestId("terminal-tab-expand"));
  expect(queryByTestId("terminal-tab-overlay") !== null).toBe(true);
});

test("Escape and an outside pointerdown both collapse the panel", () => {
  const { getByTestId, queryByTestId } = renderStrip();

  openOverlay(getByTestId);
  fireEvent.keyDown(document, { key: "Escape" });
  expect(queryByTestId("terminal-tab-overlay") === null).toBe(true);

  openOverlay(getByTestId);
  // The old strip had NO outside-press dismissal at all — only Escape —
  // so the panel could sit over the xterm surface swallowing clicks.
  fireEvent.pointerDown(document.body);
  expect(queryByTestId("terminal-tab-overlay") === null).toBe(true);
});

test("clicking an empty area inside the panel closes it", () => {
  const { getByTestId, queryByTestId } = renderStrip();

  const panel = openOverlay(getByTestId);
  fireEvent.click(panel, { target: panel });

  expect(queryByTestId("terminal-tab-overlay") === null).toBe(true);
});

test("the panel closes when the pointer moves outside its bounds", async () => {
  const { getByTestId, queryByTestId } = renderStrip();

  const panel = openOverlay(getByTestId);
  stubPanelRect(panel);

  fireEvent.pointerMove(document, { clientX: 40, clientY: 300 });
  fireEvent.pointerMove(document, { clientX: 500, clientY: 300 });
  await waitFor(() => expect(queryByTestId("terminal-tab-overlay") === null).toBe(true));
});

test("an overshoot mid-rename does not discard the rename", async () => {
  const renames: Array<[string, string]> = [];
  const { getByTestId, queryByTestId } = renderStrip({
    onRename: (id, title) => renames.push([id, title]),
  });

  const panel = openOverlay(getByTestId);
  stubPanelRect(panel);
  fireEvent.contextMenu(getByTestId("terminal-tab-row-t2"));
  fireEvent.click(getByTestId("menu-item-terminal.rename"));

  const input = getByTestId("terminal-tab-rename-input-t2");
  fireEvent.change(input, { target: { value: "Renamed" } });

  // The old strip's scheduleClose cleared `renamingId` outright, so
  // drifting the mouse away silently threw the edit on the floor.
  fireEvent.pointerMove(document, { clientX: 500, clientY: 300 });
  await act(settle);

  expect(queryByTestId("terminal-tab-overlay") !== null).toBe(true);
  expect(getByTestId("terminal-tab-rename-input-t2")).not.toBeNull();

  fireEvent.keyDown(input, { key: "Enter" });
  expect(renames).toEqual([["t2", "Renamed"]]);
});

test("the rename menu item is reachable and close is disabled for a lone terminal", () => {
  const { getByTestId } = renderStrip({ tabs: [TABS[0]!] });

  openOverlay(getByTestId);
  fireEvent.contextMenu(getByTestId("terminal-tab-row-default"));

  expect((getByTestId("menu-item-terminal.close") as HTMLButtonElement).disabled).toBe(true);
  expect((getByTestId("menu-item-terminal.rename") as HTMLButtonElement).disabled).toBe(false);
});
