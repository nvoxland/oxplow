import type { ReactElement } from "react";
import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { collectorUri, pageUri } from "../uri/uri.js";
import type { BacklinkEntry } from "./backlinkTypes.js";
import { PageNavigationContext, type PageNavigation } from "./PageNavigationContext.js";

// A backlink entry's right-click menu offers Add to Context, so the page it
// opens reaches the selected thread's basket the way a page row's does;
// the host's own entries (Unlink) come first.

const added: unknown[] = [];
const realBasket = await import("../contextBasket.js");
mock.module("../contextBasket.js", () => ({
  ...realBasket,
  addToContext: async (what: unknown) => {
    added.push(what);
  },
}));
const { BacklinksList } = await import("./BacklinksList.js");

afterEach(cleanup);

const SETTINGS = pageUri("settings");
const entry: BacklinkEntry = {
  open: SETTINGS,
  target: collectorUri("triage", "todos"),
  label: "Triage todos",
  subtitle: "about",
  linkType: "about",
};

function inPage(ui: ReactElement) {
  const nav = { navigate: () => {}, openRef: () => {} } as unknown as PageNavigation;
  return render(<PageNavigationContext.Provider value={nav}>{ui}</PageNavigationContext.Provider>);
}

test("an entry's menu offers Add to Context, which puts its page in the basket", async () => {
  const { getByTestId } = inPage(<BacklinksList entries={[entry]} />);
  const row = getByTestId(`backlinks-entry-${SETTINGS}`);
  expect(row.getAttribute("draggable")).toBeNull();
  fireEvent.contextMenu(row);
  fireEvent.click(getByTestId("menu-item-add-to-context"));
  await waitFor(() => expect(added).toEqual([{ uri: SETTINGS, label: "Triage todos" }]));
});

test("the host's own entries lead the menu", () => {
  const unlink = { id: "unlink", label: "Unlink", enabled: true, run: () => {} };
  const { getByTestId, getAllByTestId } = inPage(<BacklinksList entries={[entry]} entryMenu={() => [unlink]} />);
  fireEvent.contextMenu(getByTestId(`backlinks-entry-${SETTINGS}`));
  expect(getAllByTestId(/^menu-item-/).map((el) => el.getAttribute("data-testid"))).toEqual([
    "menu-item-unlink",
    "menu-item-add-to-context",
  ]);
});
