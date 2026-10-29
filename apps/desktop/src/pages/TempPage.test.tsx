import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { IpcCallError } from "../ipc-error.js";
import { TempPage } from "./TempPage.js";
import { fakePageRuns, tablePage } from "./testing/fakeRun.js";

afterEach(cleanup);

test("a temp page is asked of the daemon as temp/<id> and drawn like any page file", async () => {
  const run = tablePage();
  const asked: string[] = [];
  const { runs, calls } = fakePageRuns(run);
  const view = render(
    <TempPage id="v1" resource={{}} stream={null} threadId="thr1" runs={runs} getPage={async (id) => { asked.push(id); return run.page; }} />,
  );
  await view.findByText("Items: Alpha");
  expect(asked).toEqual(["temp/v1"]);
  expect(calls.open[0]).toMatchObject({ page: run.page.id, threadId: "thr1" });
});

test("a view that is gone says so, never a crash", async () => {
  const { runs, calls } = fakePageRuns(tablePage());
  const view = render(
    <TempPage id="old" resource={{}} stream={null} threadId={null} runs={runs} getPage={async () => { throw new IpcCallError("not found", "NOT_FOUND", null, []); }} />,
  );
  await view.findByText("This view is gone");
  expect(calls.open).toHaveLength(0);
});

test("a view that no longer parses says why", async () => {
  const { runs } = fakePageRuns(tablePage());
  const view = render(
    <TempPage id="bad" resource={{}} stream={null} threadId={null} runs={runs} getPage={async () => { throw new IpcCallError("view `bad`: a slot", "INVALID", null, []); }} />,
  );
  await view.findByText("view `bad`: a slot");
});
