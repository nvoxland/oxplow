import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { NewTaskPage } from "./NewTaskPage.js";

afterEach(cleanup);

// An unsaved form is the step's history state: it comes back with Back and
// a restart, and typing writes it there.
test("the new item form reads its draft from the step and writes typing back", () => {
  const writes: Record<string, unknown>[] = [];
  const nav = {
    historyState: { title: "half a title", body: "notes" },
    setHistoryState: (patch: Record<string, unknown>) => writes.push(patch),
  } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <NewTaskPage onSubmit={async () => {}} />
    </PageNavigationContext.Provider>,
  );
  expect((view.getByTestId("tasks-title") as HTMLInputElement).value).toBe("half a title");
  expect((view.getByTestId("tasks-description") as HTMLTextAreaElement).value).toBe("notes");
  fireEvent.change(view.getByTestId("tasks-title"), { target: { value: "a whole title" } });
  expect(writes).toEqual([{ title: "a whole title" }]);
});
