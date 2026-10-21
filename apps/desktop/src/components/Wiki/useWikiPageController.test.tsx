import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, render, waitFor } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "../../tabs/PageNavigationContext.js";

mock.module("../../knowledge.js", () => ({
  readWikiPage: async () => ({ page: { summary: { slug: "a" }, body: "saved" }, reads: [] }),
}));
const { useWikiPageController } = await import("./useWikiPageController.js");

afterEach(cleanup);

let controller: ReturnType<typeof useWikiPageController>;
function Probe() {
  controller = useWikiPageController("a", () => {});
  return null;
}

// An unsaved edit is the step's history state: Back and a restart bring it
// back, so a reload mid-edit loses nothing.
test("an unsaved wiki edit is read from, and written to, the step's history state", async () => {
  const states: Record<string, unknown>[] = [];
  const nav = { viewParams: {}, patchViewParams() {}, historyState: { editing: true, draft: "half-typed" }, setHistoryState: (p: Record<string, unknown>) => states.push(p) } as unknown as PageNavigation;
  render(
    <PageNavigationContext.Provider value={nav}>
      <Probe />
    </PageNavigationContext.Provider>,
  );
  await waitFor(() => expect(controller.body).toBe("saved"));
  expect(controller.editing).toBe(true);
  expect(controller.draft).toBe("half-typed");
  expect(controller.isDirty).toBe(true);
  act(() => controller.setDraft("more"));
  expect(states).toContainEqual({ draft: "more" });
  act(() => controller.revert());
  expect(states).toContainEqual({ draft: null });
});

test("with no stored draft the draft is the saved body", async () => {
  const nav = { viewParams: {}, patchViewParams() {}, historyState: {}, setHistoryState() {} } as unknown as PageNavigation;
  render(
    <PageNavigationContext.Provider value={nav}>
      <Probe />
    </PageNavigationContext.Provider>,
  );
  await waitFor(() => expect(controller.draft).toBe("saved"));
  expect(controller.isDirty).toBe(false);
  expect(controller.editing).toBe(false);
});
