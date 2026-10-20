import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "../../tabs/PageNavigationContext.js";
import type { SqlCell } from "../../tauri-bridge/generated/bindings.js";
import { tasksViewOf, tasksViewParams, useTasksView, type TasksView } from "./tasksView.js";

afterEach(cleanup);

// The Tasks page's scope and filter chips are its view params: in its
// tab's address, so two tabs differ, Back undoes one, a bookmark keeps it.
test("a Tasks view round-trips through its view params, with nothing for the defaults", () => {
  const views: TasksView[] = [
    { scope: { kind: "currentThread" }, filter: {} },
    { scope: { kind: "thread", streamId: "str1", threadId: "thr2" }, filter: { state: ["ready", "blocked"] } },
    { scope: { kind: "stream", streamId: "str3" }, filter: { priority: ["high"] } },
    { scope: { kind: "all" }, filter: {} },
  ];
  for (const view of views) expect(tasksViewOf(tasksViewParams(view))).toEqual(view);
  expect(tasksViewParams({ scope: { kind: "currentThread" }, filter: { state: [] } })).toEqual({});
  expect(tasksViewParams(views[1]!)).toEqual({ scope: "thread", stream: "str1", thread: "thr2", "f.state": "ready,blocked" });
});

test("view params that aren't a Tasks view read as its defaults", () => {
  expect(tasksViewOf({ scope: "thread", stream: "str1" })).toEqual({ scope: { kind: "currentThread" }, filter: {} });
  expect(tasksViewOf({ scope: "nope", "f.state": "" })).toEqual({ scope: { kind: "currentThread" }, filter: {} });
  expect(tasksViewOf({ other: "x" })).toEqual({ scope: { kind: "currentThread" }, filter: {} });
});

function Probe() {
  const [view, setView] = useTasksView();
  return (
    <button type="button" data-testid="probe" onClick={() => setView({ ...view, filter: { state: ["done"] } })}>
      {JSON.stringify(view)}
    </button>
  );
}

test("in a tab, the view is the tab's: a change replaces the entry and keeps other params", () => {
  const set: [Record<string, SqlCell>, unknown][] = [];
  const nav = { viewParams: { scope: "all", keep: "me" }, setViewParams: (p, o) => set.push([p, o]) } as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <Probe />
    </PageNavigationContext.Provider>,
  );
  expect(JSON.parse(view.getByTestId("probe").textContent!)).toEqual({ scope: { kind: "all" }, filter: {} });
  fireEvent.click(view.getByTestId("probe"));
  expect(set).toEqual([[{ keep: "me", scope: "all", "f.state": "done" }, undefined]]);
});

test("outside a tab, the view is the page's own", () => {
  const view = render(<Probe />);
  fireEvent.click(view.getByTestId("probe"));
  expect(JSON.parse(view.getByTestId("probe").textContent!).filter).toEqual({ state: ["done"] });
});
