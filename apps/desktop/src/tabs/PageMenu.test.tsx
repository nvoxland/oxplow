import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { personSpec, recordRefOffers } from "../components/refCommandsTestSupport.js";

// The nav bar's ☰ lists the commands about the page's ref the daemon says
// apply to it now (`refCommands`: their `ui.about`, and `ui.about_when` held
// on its row), at its top level, and runs one, the ref bound into its input;
// and Ask the Assistant opens the Assistant about the page.

// What the daemon answers (`ref_commands`) for a ref: its kind's commands,
// less those whose `about_when` fails on its row (`notNow`).
const notNow = new Set<string>();
const KINDS: Record<string, string[]> = {
  "oxplow:oxplow/work_item": ["tracker.item.estimate"],
  "oxplow:oxplow/commit": ["tracker.commit.sync"],
  "oxplow:oxplow/effort": ["oxplow.review.accept", "oxplow.review.request_changes"],
  "oxplow:oxplow/agent_session": ["oxplow.agent_session.reopen"],
};

const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  refCommands: async (ref: string) => (KINDS[ref.split("?")[0]] ?? []).filter((id) => !notNow.has(id)),
  listPersonCommands: async () => [
    personSpec("tracker.item.estimate", { label: "Estimate in Fake…", group: "Tracker", about: "oxplow/work_item", input: { uri: "{{uri}}", points: 3 } }),
    personSpec("tracker.commit.sync", { label: "Sync", group: "Tracker", about: "oxplow/commit", input: { sha: "{{uri.id}}" } }),
    personSpec("oxplow.review.accept", { label: "Accept Review", group: "Review", about: "oxplow/effort", input: { uri: "{{uri}}" } }),
    personSpec("oxplow.review.request_changes", { label: "Request Changes", group: "Review", about: "oxplow/effort", input: { uri: "{{uri}}" } }),
    personSpec("oxplow.agent_session.reopen", {
      label: "Reopen Session",
      group: "Tasks",
      about: "oxplow/agent_session",
      about_when: "reopenable",
      input: { session: "{{uri.id}}" },
    }),
  ],
}));
const { PageMenu } = await import("./PageMenu.js");
const { dockStore } = await import("../components/Dock/dockStore.js");
const { setAssistantScope } = await import("../components/Assistant/assistantSession.js");

const ran: Array<[string, unknown]> = [];
beforeEach(() => {
  ran.length = 0;
  notNow.clear();
  recordRefOffers(ran);
});
afterEach(cleanup);

/** Open `ref`'s ☰. */
async function openMenu(ref: string) {
  const view = render(<PageMenu ask={{ ref }} buttonStyle={{}} />);
  fireEvent.click(await waitFor(() => view.getByTestId("page-nav-menu")));
  await waitFor(() => view.getByTestId("menu-item-page-ask-assistant"));
  return view;
}

test("a page's ref gets its kind's commands at the menu's top level, run with the ref bound", async () => {
  const view = await openMenu("oxplow:oxplow/work_item?id=W-1&provider=fake");
  // The command's `ui.group` (Tracker) makes no submenu here.
  expect(view.container.querySelector('[data-testid^="menu-item-page-submenu-"]')).toBeNull();
  expect(view.queryByTestId("menu-item-ref-command-tracker.commit.sync")).toBeNull();
  fireEvent.click(view.getByTestId("menu-item-ref-command-tracker.item.estimate"));
  await waitFor(() => expect(ran).toEqual([["tracker.item.estimate", { uri: "oxplow:oxplow/work_item?id=W-1&provider=fake", points: 3 }]]));
  // Running one closes the menu.
  await waitFor(() => expect(view.queryByTestId("menu-item-ref-command-tracker.item.estimate")).toBeNull());
});

test("a command the daemon says doesn't apply to the ref's row is not offered", async () => {
  // Reopen Session: on a session a person closed, not an open one.
  notNow.add("oxplow.agent_session.reopen");
  const open = await openMenu("oxplow:oxplow/agent_session?id=ses1");
  expect(open.queryByTestId("menu-item-ref-command-oxplow.agent_session.reopen")).toBeNull();
  expect(open.container.querySelector('[data-testid^="menu-item-page-submenu-"]')).toBeNull();
  cleanup();
  notNow.clear();
  const closed = await openMenu("oxplow:oxplow/agent_session?id=ses2");
  fireEvent.click(closed.getByTestId("menu-item-ref-command-oxplow.agent_session.reopen"));
  await waitFor(() => expect(ran).toEqual([["oxplow.agent_session.reopen", { session: "ses2" }]]));
});

test("no commands for the page's kind: the menu has Ask and no commands", async () => {
  const view = await openMenu("oxplow:oxplow/wiki?slug=notes");
  expect(view.getByTestId("menu-item-page-ask-assistant")).toBeTruthy();
  expect(view.container.querySelector('[data-testid^="menu-item-ref-command-"]')).toBeNull();
});

// An effort's page (its diff, `oxplow:oxplow/effort?id=effN`) offers oxplow-bundled's
// verdicts, each run on that effort.
test("an effort's page offers Accept Review and Request Changes", async () => {
  const view = await openMenu("oxplow:oxplow/effort?id=eff3");
  expect(view.getByTestId("menu-item-ref-command-oxplow.review.request_changes")).toBeTruthy();
  fireEvent.click(view.getByTestId("menu-item-ref-command-oxplow.review.accept"));
  await waitFor(() => expect(ran).toEqual([["oxplow.review.accept", { uri: "oxplow:oxplow/effort?id=eff3" }]]));
});

// Ask the Assistant: the dock's Assistant panel opens, about the page.
test("Ask the Assistant opens the Assistant panel about the page", async () => {
  setAssistantScope({ stream: null, thread: null });
  const view = await openMenu("oxplow:oxplow/commit?sha=abc123");
  fireEvent.click(view.getByTestId("menu-item-page-ask-assistant"));
  await waitFor(() => expect(dockStore.get().open).toBe("assistant"));
  dockStore.close();
});

test("the ☰ button closes the menu it opened", async () => {
  const view = await openMenu("oxplow:oxplow/commit?sha=abc123");
  const button = view.getByTestId("page-nav-menu");
  fireEvent.mouseDown(button);
  fireEvent.click(button);
  expect(view.queryByTestId("menu-item-page-ask-assistant")).toBeNull();
});

test("a page with nothing to offer has no ☰", () => {
  const view = render(<PageMenu buttonStyle={{}} />);
  expect(view.queryByTestId("page-nav-menu")).toBeNull();
});

test("Backlinks opens its list under the ☰, and Escape closes it", async () => {
  const view = render(<PageMenu backlinks={{ count: 1, body: <span>linked from A</span> }} buttonStyle={{}} />);
  fireEvent.click(view.getByTestId("page-nav-menu"));
  fireEvent.click(view.getByTestId("menu-item-page-backlinks"));
  await waitFor(() => expect(view.getByTestId("page-nav-backlinks-popover").textContent).toBe("linked from A"));
  fireEvent.keyDown(window, { key: "Escape" });
  expect(view.queryByTestId("page-nav-backlinks-popover")).toBeNull();
});
