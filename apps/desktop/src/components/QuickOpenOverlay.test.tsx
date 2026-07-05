import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

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
      menuGroups={[]}
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
