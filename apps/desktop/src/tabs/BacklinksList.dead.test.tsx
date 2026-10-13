import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { BacklinksList } from "./BacklinksList.js";
import type { BacklinkEntry } from "./backlinkTypes.js";
import { indexRef } from "./pageRefs.js";

afterEach(cleanup);

const collector: BacklinkEntry = {
  ref: indexRef("settings"),
  target: "collector:triage/todos",
  label: "Triage todos",
  subtitle: "about",
  linkType: "about",
};
const dead: BacklinkEntry = {
  ref: null,
  target: "acme_pr:12",
  label: "acme_pr:12",
  subtitle: "about",
  dead: true,
  linkType: "about",
};

test("a link to a collector opens its page", () => {
  const onOpenPage = mock(() => {});
  const { getByTestId } = render(<BacklinksList entries={[collector]} onOpenPage={onOpenPage} />);
  fireEvent.click(getByTestId(`backlinks-entry-${collector.ref!.id}`), { metaKey: true });
  expect(onOpenPage).toHaveBeenCalledWith(collector.ref);
});

test("a dead link shows its raw ref, muted, with the reason, and opens nothing", () => {
  const onOpenPage = mock(() => {});
  const { getByTestId } = render(<BacklinksList entries={[dead]} onOpenPage={onOpenPage} />);
  const row = getByTestId("backlinks-dead-acme_pr:12");
  expect(row.textContent).toContain("acme_pr:12");
  expect(row.getAttribute("title")).toBe("nothing resolves this ref; its extension may be gone");
  fireEvent.click(row);
  fireEvent.contextMenu(row);
  expect(onOpenPage).not.toHaveBeenCalled();
});

test("right-click on a link row offers its actions, Unlink among them, even when dead", () => {
  const unlink = mock(() => {});
  const { getByTestId } = render(
    <BacklinksList
      entries={[dead]}
      onOpenPage={() => {}}
      entryMenu={() => [{ id: "unlink", label: "Unlink", enabled: true, run: unlink }]}
    />,
  );
  fireEvent.contextMenu(getByTestId("backlinks-dead-acme_pr:12"), { clientX: 5, clientY: 5 });
  fireEvent.click(getByTestId("menu-item-unlink"));
  expect(unlink).toHaveBeenCalled();
});
