import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { personSpec, recordRefOffers } from "../components/refCommandsTestSupport.js";

// The nav bar's ☰ lists the commands about the page's ref (their
// `ui.about`, one submenu per group) and runs one, the ref bound into its
// input; and Ask About This puts the page's ref in the agent's input.

const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  listPersonCommands: async () => [
    personSpec("tracker.item.estimate", { label: "Estimate in Fake…", group: "Tracker", about: "oxplow/work_item", input: { uri: "{{uri}}", points: 3 } }),
    personSpec("tracker.commit.sync", { label: "Sync", group: "Tracker", about: "oxplow/commit", input: { sha: "{{uri.id}}" } }),
    personSpec("oxplow.review.accept", { label: "Accept Review", group: "Review", about: "oxplow/effort", input: { uri: "{{uri}}" } }),
    personSpec("oxplow.review.request_changes", { label: "Request Changes", group: "Review", about: "oxplow/effort", input: { uri: "{{uri}}" } }),
  ],
}));
const { PageMenu } = await import("./PageMenu.js");
const { subscribeAgentInput } = await import("../agent-input-bus.js");

const ran: Array<[string, unknown]> = [];
beforeEach(() => {
  ran.length = 0;
  recordRefOffers(ran);
});
afterEach(cleanup);

/** Open `ref`'s ☰ and the submenu `item` (a click opens one, as hover does). */
async function openMenu(ref: string, item?: string) {
  const view = render(<PageMenu ask={{ ref }} buttonStyle={{}} />);
  fireEvent.click(await waitFor(() => view.getByTestId("page-nav-menu")));
  if (item) fireEvent.click(await waitFor(() => view.getByTestId(`menu-item-${item}`)));
  return view;
}

test("a page's ref gets its kind's commands, run with the ref bound", async () => {
  const view = await openMenu("oxplow:oxplow/work_item?id=W-1&provider=fake", "ref-commands-Tracker");
  expect(view.queryByTestId("menu-item-ref-command-tracker.commit.sync")).toBeNull();
  fireEvent.click(view.getByTestId("menu-item-ref-command-tracker.item.estimate"));
  await waitFor(() => expect(ran).toEqual([["tracker.item.estimate", { uri: "oxplow:oxplow/work_item?id=W-1&provider=fake", points: 3 }]]));
  // Running one closes the menu.
  await waitFor(() => expect(view.queryByTestId("menu-item-ref-commands-Tracker")).toBeNull());
});

test("no commands for the page's kind: the menu has Ask and no command groups", async () => {
  const view = await openMenu("oxplow:oxplow/wiki?slug=notes");
  await new Promise((r) => setTimeout(r, 20));
  expect(view.getByTestId("menu-item-page-ask")).toBeTruthy();
  expect(view.container.querySelector('[data-testid^="menu-item-ref-commands-"]')).toBeNull();
});

// An effort's page (its diff, `oxplow:oxplow/effort?id=effN`) offers oxplow-bundled's
// verdicts, each run on that effort.
test("an effort's page offers Accept Review and Request Changes", async () => {
  const view = await openMenu("oxplow:oxplow/effort?id=eff3", "ref-commands-Review");
  expect(view.getByTestId("menu-item-ref-command-oxplow.review.request_changes")).toBeTruthy();
  fireEvent.click(view.getByTestId("menu-item-ref-command-oxplow.review.accept"));
  await waitFor(() => expect(ran).toEqual([["oxplow.review.accept", { uri: "oxplow:oxplow/effort?id=eff3" }]]));
});

// Ask About This: the open page's ref goes into the agent's input for the
// person to finish the question — never sent.
test("Ask About This puts the page's ref in the agent's input", async () => {
  const inserted: string[] = [];
  const off = subscribeAgentInput((t) => inserted.push(t));
  const view = await openMenu("oxplow:oxplow/commit?sha=abc123", "page-ask");
  fireEvent.click(view.getByTestId("menu-item-page-ask-this"));
  await waitFor(() => expect(inserted).toEqual(["[oxplow uri oxplow:oxplow/commit?sha=abc123] "]));
  off();
});

test("the ☰ button closes the menu it opened", async () => {
  const view = await openMenu("oxplow:oxplow/commit?sha=abc123");
  const button = view.getByTestId("page-nav-menu");
  fireEvent.mouseDown(button);
  fireEvent.click(button);
  expect(view.queryByTestId("menu-item-page-ask")).toBeNull();
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
