import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import type { VizRun } from "../api.js";
import { createRememberedStore, RememberedContext } from "../prefs/remembered.js";
import { FormSourceContext, type FormSource } from "./formSource.js";

const realApi = { ...(await import("../api.js")) };
mock.module("../api.js", () => ({ ...realApi, commandOpenAfter: async () => null }));
const { VizView } = await import("./VizView.js");
afterEach(cleanup);
afterAll(() => {
  mock.module("../api.js", () => realApi);
});
const run = { config: { id: "session-picker", title: "Start a new session", empty: null, viz: "form", columns: [], actions: [], params: [], form: { command: "oxplow.agent_session.open", mode: "create", then: "stay" } }, params: {}, result: { columns: [], rows: [] } } as unknown as VizRun;
const choices = [{ value: "claude", label: "Claude", group: null }, { value: "codex", label: "Codex", group: null }];
function source(submitted: unknown[], enabled = choices): FormSource {
  return {
    start: async () => ({ command: { id: "oxplow.agent_session.open", input_schema: { type: "object", required: ["harness"], properties: { harness: { type: "string" } } } }, values: { harness: enabled[0]?.value ?? "" }, fields: ["harness"], fixed: [], choices: { harness: enabled }, reads: { datasets: [], tables: [], measures: [] } }) as never,
    submit: async (_target, input) => { submitted.push(input); return { result: null, command_run_id: null, event_id: null, inverse: null }; },
    actionStart: async () => { throw new Error("not an action form"); },
  };
}

test("session picker remembers the harness after successful explicit submission and ignores disabled choices", async () => {
  const submitted: unknown[] = [];
  const store = createRememberedStore({ load: async () => [], save: async () => {} }, 0);
  await store.ready;
  const formSource = source(submitted);
  const ui = render(<RememberedContext.Provider value={store}><FormSourceContext.Provider value={formSource}><VizView run={run} onOpenPage={() => {}} /></FormSourceContext.Provider></RememberedContext.Provider>);
  const field = await waitFor(() => ui.getByTestId("viz-form-fields-harness") as HTMLSelectElement);
  fireEvent.change(field, { target: { value: "codex" } });
  expect(submitted.length).toBe(0);
  fireEvent.submit(ui.getByTestId("viz-form-fields"));
  await waitFor(() => expect(submitted).toEqual([{ harness: "codex" }]));
  await waitFor(() => expect((ui.getByTestId("viz-form-fields-harness") as HTMLSelectElement).value).toBe("codex"));
  ui.unmount();
  const next = render(<RememberedContext.Provider value={store}><FormSourceContext.Provider value={source(submitted, [choices[0]!])}><VizView run={run} onOpenPage={() => {}} /></FormSourceContext.Provider></RememberedContext.Provider>);
  await waitFor(() => expect((next.getByTestId("viz-form-fields-harness") as HTMLSelectElement).value).toBe("claude"));
  expect(submitted.length).toBe(1);
});

test("a saved picker preference arriving after form start updates preselection without opening a session", async () => {
  let resolveLoad!: (rows: { scope: string; key: string; valueJson: string }[]) => void;
  const store = createRememberedStore({ load: () => new Promise((resolve) => { resolveLoad = resolve; }), save: async () => {} }, 0);
  const submitted: unknown[] = [];
  const ui = render(<RememberedContext.Provider value={store}><FormSourceContext.Provider value={source(submitted)}><VizView run={run} onOpenPage={() => {}} /></FormSourceContext.Provider></RememberedContext.Provider>);
  await waitFor(() => expect((ui.getByTestId("viz-form-fields-harness") as HTMLSelectElement).value).toBe("claude"));
  resolveLoad([{ scope: "app", key: "session-picker.last-harness", valueJson: '"codex"' }]);
  await waitFor(() => expect((ui.getByTestId("viz-form-fields-harness") as HTMLSelectElement).value).toBe("codex"));
  expect(submitted.length).toBe(0);
});
