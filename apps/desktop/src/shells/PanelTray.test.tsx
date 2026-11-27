import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { ExtensionPanel } from "../tauri-bridge/generated/bindings.js";
import { PanelTrayView, PinnedPanelView, type TrayPanel } from "./PanelTray.js";

afterEach(cleanup);

const panel = (id: string, title: string): ExtensionPanel =>
  ({ id, extension: "x", title, icon: null, dataScope: "thread", body: `x/${id}`, badge: null, open: null, collapsed: null, count: null }) as unknown as ExtensionPanel;
const tray = (id: string, title: string, count: number | null = null): TrayPanel => ({
  id: `ext:x/${id}`,
  panel: panel(`x/${id}`, title),
  runs: { body: null, badge: null, collapsed: null, count, choices: [] },
  open: { id: `oxplow:oxplow/page?page=x/${id}`, kind: "page" } as unknown as TrayPanel["open"],
});

function view(over: { pinned?: string | null } = {}) {
  const pins: (string | null)[] = [];
  const v = render(
    <PanelTrayView
      panels={[tray("waiting", "Waiting on you", 3), tray("work", "Work")]}
      choose={() => {}}
      pinned={over.pinned ?? null}
      setPinned={(id) => pins.push(id)}
      streamId={null}
      onOpenPage={() => {}}
    />,
  );
  return { v, pins };
}

// The tray: each rail panel by name and count, in the rail's order, then the dock.
test("the tray lists the rail's panels with their counts", () => {
  const { v } = view();
  const names = Array.from(v.getByTestId("tablet-panels").querySelectorAll("button")).map((b) => b.textContent);
  expect(names).toEqual(["Waiting on you3", "Work"]);
  expect(v.getByTestId("tablet-panel-count-x/waiting").textContent).toBe("3");
});

// A tap opens the panel as a sheet; its pin keeps it beside the page.
test("a tap opens the panel's sheet; Pin pins it and closes the sheet", () => {
  const { v, pins } = view();
  fireEvent.click(v.getByTestId("tablet-panel-x/work"));
  expect(v.getByTestId("tablet-panel-sheet-title").textContent).toBe("Work");
  fireEvent.click(v.getByTestId("tablet-panel-pin"));
  expect(pins).toEqual(["ext:x/work"]);
  expect(v.queryByTestId("tablet-panel-sheet")).toBeNull();
});

test("the pinned panel's button is pressed; its sheet's pin unpins", () => {
  const { v, pins } = view({ pinned: "ext:x/work" });
  expect(v.getByTestId("tablet-panel-x/work").getAttribute("aria-pressed")).toBe("true");
  fireEvent.click(v.getByTestId("tablet-panel-x/work"));
  fireEvent.click(v.getByTestId("tablet-panel-pin"));
  expect(pins).toEqual([null]);
});

test("the pinned panel is a column with its name and an unpin; none draws nothing", () => {
  let unpinned = 0;
  const v = render(<PinnedPanelView panel={tray("work", "Work")} choose={() => {}} unpin={() => unpinned++} streamId={null} onOpenPage={() => {}} />);
  expect(v.getByTestId("tablet-pinned-panel").getAttribute("aria-label")).toBe("Work");
  fireEvent.click(v.getByTestId("tablet-panel-pin"));
  expect(unpinned).toBe(1);
  cleanup();
  const none = render(<PinnedPanelView panel={null} choose={() => {}} unpin={() => {}} streamId={null} onOpenPage={() => {}} />);
  expect(none.container.innerHTML).toBe("");
});
