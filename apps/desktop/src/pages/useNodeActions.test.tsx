import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { PageActionContext, nodeActionRunner, nodeFormSource, useNodeActions, type RunPageAction } from "./useNodeActions.js";
import type { LensActionRunner } from "../lens/lensActionRunner.js";

afterEach(cleanup);

const recorder = () => {
  const calls: unknown[][] = [];
  const press: RunPageAction = async (...args) => {
    calls.push(args);
    return { result: null, audit_id: null, event_id: null, inverse: null } as never;
  };
  return { calls, press };
};

test("a viz's press runs the node's action on the open run, with the row, input and drop", async () => {
  const { calls, press } = recorder();
  const run = nodeActionRunner(press, "run-1", "notes/items");
  await run("ignored/lens", "finish", { p: 1 }, { id: "a" }, "stream", true, { note: "x" }, { onto: { id: "b" } });
  expect(calls).toEqual([["run-1", "notes/items", "finish", { id: "a" }, { note: "x" }, { onto: { id: "b" } }, true]]);
});

test("input and drop default to none; a whole-page action names no node", async () => {
  const { calls, press } = recorder();
  await nodeActionRunner(press, "run-1", null)("x", "add", {}, null, null, false);
  expect(calls).toEqual([["run-1", null, "add", null, null, null, false]]);
});

test("before the run opens a press fails, saying so", async () => {
  const { calls, press } = recorder();
  await expect(nodeActionRunner(press, null, "items")("x", "add", {}, null, null, false)).rejects.toThrow("isn't open");
  expect(calls).toEqual([]);
});

test("useNodeActions presses through the page's action context", async () => {
  const { calls, press } = recorder();
  let runner: LensActionRunner | null = null;
  function Probe() {
    runner = useNodeActions("run-9", "items");
    return null;
  }
  render(
    <PageActionContext.Provider value={press}>
      <Probe />
    </PageActionContext.Provider>,
  );
  await runner!("x", "go", {}, { id: "a" }, null, false);
  expect(calls).toEqual([["run-9", "items", "go", { id: "a" }, null, null, false]]);
});

// What a command changed shows even when no model change announces it
// (a keychain entry): the run is run again after the press.
test("a press that ran runs its run again; one that failed doesn't", async () => {
  const { press } = recorder();
  const after: string[] = [];
  await nodeActionRunner(press, "run-1", "items", (id) => after.push(id))("x", "set", {}, { id: "a" }, null, false);
  expect(after).toEqual(["run-1"]);
  const failing: RunPageAction = async () => {
    throw new Error("refused");
  };
  await expect(nodeActionRunner(failing, "run-1", "items", (id) => after.push(id))("x", "set", {}, null, null, false)).rejects.toThrow("refused");
  expect(after).toEqual(["run-1"]);
});

test("a submitted form runs its run again", async () => {
  const after: string[] = [];
  const forms = {
    start: async () => ({}) as never,
    submit: async () => ({ result: null, audit_id: null, event_id: null, inverse: null }) as never,
    actionStart: async () => ({}) as never,
  };
  await nodeFormSource(forms, "run-2", "form", (id) => after.push(id)).submit({ config: { id: "x" }, params: {} }, {}, null, false);
  expect(after).toEqual(["run-2"]);
});
