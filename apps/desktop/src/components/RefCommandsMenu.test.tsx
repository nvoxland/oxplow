import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { personSpec, recordRefOffers } from "./refCommandsTestSupport.js";

// The nav bar's Commands lists the commands about the page's ref (their
// `ui.about`) and runs one, the ref bound into its input.

const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  listPersonCommands: async () => [
    personSpec("tracker.item.estimate", { label: "Estimate in Fake…", group: "Tracker", about: "work_item", input: { ref: "{{ref}}", points: 3 } }),
    personSpec("tracker.commit.sync", { label: "Sync", group: "Tracker", about: "commit", input: { sha: "{{ref.id}}" } }),
    personSpec("oxplow.review.accept", { label: "Accept Review", group: "Review", about: "effort", input: { ref: "{{ref}}" } }),
    personSpec("oxplow.review.request_changes", { label: "Request Changes", group: "Review", about: "effort", input: { ref: "{{ref}}" } }),
  ],
}));
const { RefCommandsMenu } = await import("./RefCommandsMenu.js");

const ran: Array<[string, unknown]> = [];
beforeEach(() => {
  ran.length = 0;
  recordRefOffers(ran);
});
afterEach(cleanup);

test("a page's ref gets its kind's commands, run with the ref bound", async () => {
  const view = render(<RefCommandsMenu target={{ ref: "work_item:fake:W-1", streamId: null }} buttonStyle={{}} />);
  fireEvent.click(await waitFor(() => view.getByTestId("page-nav-commands")));
  expect(view.getByTestId("page-nav-commands-menu").textContent).toContain("Tracker");
  expect(view.queryByTestId("page-nav-command-tracker.commit.sync")).toBeNull();
  fireEvent.click(view.getByTestId("page-nav-command-tracker.item.estimate"));
  await waitFor(() => expect(ran).toEqual([["tracker.item.estimate", { ref: "work_item:fake:W-1", points: 3 }]]));
});

test("no commands for the page's kind, no menu", async () => {
  const view = render(<RefCommandsMenu target={{ ref: "wiki:notes", streamId: null }} buttonStyle={{}} />);
  await new Promise((r) => setTimeout(r, 20));
  expect(view.queryByTestId("page-nav-commands")).toBeNull();
});

// P7.C5: an effort's page (its diff, `effort:effN`) offers oxplow-bundled's
// verdicts, each run on that effort.
test("an effort's page offers Accept Review and Request Changes", async () => {
  const view = render(<RefCommandsMenu target={{ ref: "effort:eff3", streamId: null }} buttonStyle={{}} />);
  fireEvent.click(await waitFor(() => view.getByTestId("page-nav-commands")));
  const menu = view.getByTestId("page-nav-commands-menu").textContent ?? "";
  expect(menu).toContain("Accept Review");
  expect(menu).toContain("Request Changes");
  fireEvent.click(view.getByTestId("page-nav-command-oxplow.review.accept"));
  await waitFor(() => expect(ran).toEqual([["oxplow.review.accept", { ref: "effort:eff3" }]]));
});
