import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import type { Lens, LensRun } from "../tauri-bridge/generated/bindings.js";
import { IpcCallError } from "../ipc-error.js";

const realApi = await import("../api.js");
// Bun's `mock.module` is process-wide: everything not faked here delegates
// to the real module so other test files keep working.
const calls: boolean[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  runLensAction: async (
    _id: string,
    _action: string,
    _params: unknown,
    _row: unknown,
    _streamId: unknown,
    confirmed: boolean,
  ) => {
    calls.push(confirmed);
    if (!confirmed) throw new IpcCallError("Finish needs confirmation", "NEEDS_CONFIRMATION");
    return { result: null, auditId: 1, eventId: null, inverse: null };
  },
  getCommand: async () => Promise.reject(new Error("no bridge in tests")),
}));

const { LensResultView } = await import("./LensResultView.js");

afterEach(cleanup);

const lens: Lens = {
  id: "x/l",
  extension: "x",
  slug: "l",
  title: "L",
  description: "",
  query: "",
  viz: "table",
  params: [],
  columns: [],
  empty: null,
  chart: null,
  tree: null,
  timeline: null,
  steps: null,
  hunks: null,
  form: null,
  children: [],
  launcherCategory: null,
  hidden: false,
  actions: [{ id: "finish", label: "Finish", command: "work_item.transition", input: {}, row: true }],
  alert: null,
  path: "",
};
const run: LensRun = {
  lens,
  params: {},
  result: { columns: ["ref"], rows: [["work_item:oxplow:tsk1"]], truncated: false },
};

// A row action that asks for confirmation must ask wherever the row is —
// an answer in the strip, a grid child, a dashboard tile — not only where
// the toolbar is shown.
test("a row action's confirmation shows when the toolbar is hidden", async () => {
  const { getByTestId, queryByTestId } = render(<LensResultView run={run} toolbar={false} onOpenPage={() => {}} />);
  fireEvent.contextMenu(getByTestId("lens-row-0"));
  fireEvent.click(getByTestId("menu-item-lens-action-finish"));
  await waitFor(() => expect(calls).toEqual([false]));
  await waitFor(() => expect(queryByTestId("lens-action-confirm")).not.toBeNull());
  fireEvent.click(getByTestId("lens-action-confirm-run"));
  await waitFor(() => expect(calls).toEqual([false, true]));
  await waitFor(() => expect(queryByTestId("lens-action-confirm")).toBeNull());
});
