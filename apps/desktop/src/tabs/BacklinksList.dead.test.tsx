import type { ReactElement } from "react";
import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { collectorUri, pageUri, resourceUri } from "../uri/uri.js";
import { BacklinksList } from "./BacklinksList.js";
import type { BacklinkEntry } from "./backlinkTypes.js";
import { PageNavigationContext, type PageNavigation } from "./PageNavigationContext.js";

afterEach(cleanup);

const DEAD_REF = resourceUri("acme/pr", { id: 12 });
const SETTINGS = pageUri("settings");
const collector: BacklinkEntry = {
  open: SETTINGS,
  target: collectorUri("triage", "todos"),
  label: "Triage todos",
  subtitle: "about",
  linkType: "about",
};
const dead: BacklinkEntry = {
  open: null,
  target: DEAD_REF,
  label: DEAD_REF,
  subtitle: "about",
  dead: true,
  linkType: "about",
};

/** A page that records what its links open, and how. */
function inPage(ui: ReactElement, opened: Array<[string, boolean | undefined]>) {
  const nav = {
    navigate: () => {},
    openRef: (ref: string, how?: { newTab?: boolean }) => opened.push([ref, how?.newTab]),
  } as unknown as PageNavigation;
  return render(<PageNavigationContext.Provider value={nav}>{ui}</PageNavigationContext.Provider>);
}

test("a link to a collector opens its page, in this tab or a new one", () => {
  const opened: Array<[string, boolean | undefined]> = [];
  const { getByTestId } = inPage(<BacklinksList entries={[collector]} />, opened);
  fireEvent.click(getByTestId(`backlinks-entry-${SETTINGS}`));
  fireEvent.click(getByTestId(`backlinks-entry-${SETTINGS}`), { metaKey: true });
  expect(opened).toEqual([
    [SETTINGS, undefined],
    [SETTINGS, true],
  ]);
});

test("a dead link shows its raw ref, muted, with the reason, and opens nothing", () => {
  const opened: Array<[string, boolean | undefined]> = [];
  const { getByTestId } = inPage(<BacklinksList entries={[dead]} />, opened);
  const row = getByTestId(`backlinks-dead-${DEAD_REF}`);
  expect(row.textContent).toContain(DEAD_REF);
  expect(row.getAttribute("title")).toBe("nothing resolves this ref; its extension may be gone");
  fireEvent.click(row);
  fireEvent.contextMenu(row);
  expect(opened).toEqual([]);
});

test("right-click on a link row offers its actions, Unlink among them, even when dead", () => {
  const unlink = mock(() => {});
  const { getByTestId } = render(
    <BacklinksList
      entries={[dead]}
      entryMenu={() => [{ id: "unlink", label: "Unlink", enabled: true, run: unlink }]}
    />,
  );
  fireEvent.contextMenu(getByTestId(`backlinks-dead-${DEAD_REF}`), { clientX: 5, clientY: 5 });
  fireEvent.click(getByTestId("menu-item-unlink"));
  expect(unlink).toHaveBeenCalled();
});
