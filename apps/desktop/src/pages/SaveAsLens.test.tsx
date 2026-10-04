import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Stream } from "../api.js";
import type { TabRef } from "../tabs/tabState.js";

// tsk943: Explore Data's Save as Lens is the `lens.keep` command with a
// spec — on the bus like Keep This, in the stream's worktree — and the
// lens it kept opens.

const realApi = await import("../api.js");
const ran: Array<[string, unknown]> = [];
mock.module("../api.js", () => ({
  ...realApi,
  runCommand: async (name: string, input: unknown) => {
    ran.push([name, input]);
    return { result: { lens: "mine/busy-tasks", path: "oxplow/extensions/mine/lenses/busy-tasks.yaml" } };
  },
}));

const { SaveAsLens } = await import("./ExploreDataPage.js");

afterEach(() => {
  cleanup();
  ran.length = 0;
});

test("Save as Lens keeps the spec with lens.keep and opens the lens", async () => {
  const opened: TabRef[] = [];
  const view = render(
    <SaveAsLens
      query="SELECT title FROM v_task"
      viz="table"
      stream={{ id: "str2" } as Stream}
      onOpenPage={(ref) => opened.push(ref)}
      disabledReason={null}
    />,
  );
  fireEvent.click(view.getByTestId("explore-save-open"));
  fireEvent.change(view.getByTestId("explore-save-title"), { target: { value: "Busy Tasks" } });
  fireEvent.keyDown(view.getByTestId("explore-save-title"), { key: "Enter" });
  await waitFor(() => expect(opened.map((r) => r.id)).toEqual(["lens:mine/busy-tasks"]));
  expect(ran).toEqual([
    [
      "lens.keep",
      {
        spec: { title: "Busy Tasks", description: "", query: "SELECT title FROM v_task", viz: "table" },
        extension: "mine",
        slug: "busy-tasks",
        stream: "str2",
      },
    ],
  ]);
});
