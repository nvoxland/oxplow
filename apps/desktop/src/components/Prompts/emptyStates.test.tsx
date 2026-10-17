import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import type { Stream, Thread } from "../../api.js";

// `.context/usability.md` → "Empty states": every empty page or section is
// an `EmptyState` (one mechanism for empty copy), which marks its root
// with `data-empty-state`. These mount the surfaces that can be mounted
// cheaply and look for that mark where the old plain copy was.

const realApi = await import("../../api.js");
mock.module("../../api.js", () => ({
  ...realApi,
  subscribeOxplowEvents: () => () => {},
  listRecentPageVisits: async () => [],
  topVisitedPages: async () => [],
  subscribePageVisitEvents: () => () => {},
  listMetricCatalog: async () => ({ rows: [], reads: { models: [], tables: [], measures: [] } }),
  listMetricDefinitions: async () => ({ rows: [], reads: { models: [], tables: [], measures: [] } }),
  listMetricSamples: async () => ({ rows: [], reads: { models: [], tables: [], measures: [] } }),
}));

const { BacklinksList } = await import("../../tabs/BacklinksList.js");
const { DashboardPage } = await import("../../pages/DashboardPage.js");
const { MetricsPage } = await import("../../pages/MetricsPage.js");
const { NewSessionPage } = await import("../../pages/NewSessionPage.js");

afterEach(cleanup);

const stream = { id: "str1", name: "main", kind: "primary" } as unknown as Stream;
const emptyStates = (container: HTMLElement) => Array.from(container.querySelectorAll("[data-empty-state]"));

test("an empty backlinks list is an EmptyState", () => {
  const { container } = render(<BacklinksList entries={[]} onOpenPage={() => {}} />);
  const empty = container.querySelector('[data-testid="backlinks-list-empty"]');
  expect(empty?.hasAttribute("data-empty-state")).toBe(true);
});

test("the Go To page's empty sections are EmptyStates", async () => {
  const { container } = render(<DashboardPage stream={stream} onOpenPage={() => {}} />);
  await waitFor(() => expect(container.textContent).toContain("No visits"));
  // Bookmarks and visits, each an EmptyState.
  expect(emptyStates(container).length).toBeGreaterThanOrEqual(2);
});

test("no metrics recorded is an EmptyState", async () => {
  const { container } = render(<MetricsPage />);
  await waitFor(() => expect(container.textContent).toContain("No metrics recorded"));
  expect(emptyStates(container).length).toBe(1);
});

/** The session picker offers no prompts: there is no agent to hand one
 *  to yet. */
test("a thread with no agent session shows the picker, with no prompts", () => {
  const thread = { id: "thr1", title: "Fix the cart" } as unknown as Thread;
  const { container, getByTestId } = render(
    <NewSessionPage
      thread={thread}
      harnesses={[
        { id: "claude", title: "Claude", enabled: true },
        { id: "codex", title: "Codex", enabled: true },
        { id: "opencode", title: "OpenCode", enabled: false },
      ]}
      onStart={async () => {}}
    />,
  );
  expect(container.textContent).not.toContain("Ask the agent");
  expect(Array.from((getByTestId("new-session-agent") as HTMLSelectElement).options).map((o) => o.value)).toEqual([
    "claude",
    "codex",
  ]);
});
