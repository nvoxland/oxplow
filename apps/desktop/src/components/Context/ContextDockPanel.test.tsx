import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { SqlCell, SqlQueryResult } from "../../tauri-bridge/generated/bindings.js";

const ran: Array<[string, unknown]> = [];
let rows: SqlCell[][] = [];
const realApi = await import("../../api.js");
mock.module("../../api.js", () => ({
  ...realApi,
  querySql: async () =>
    ({
      columns: ["id", "kind", "source_uri", "label", "text", "delivered_at"],
      rows,
      truncated: false,
      reads: { datasets: [], tables: [], measures: [] },
      freshness: {},
    }) as unknown as SqlQueryResult,
  runCommand: async (name: string, input: unknown) => {
    ran.push([name, input]);
    return { audit_id: 1, inverse: null, result: {} };
  },
}));
const { setSelectedThread } = await import("../../contextBasket.js");
const { ContextDockBody } = await import("./ContextDockPanel.js");

afterEach(() => {
  cleanup();
  ran.length = 0;
  setSelectedThread(null);
});

test("it lists what waits, takes an item out and clears the lot", async () => {
  setSelectedThread("thr7");
  rows = [
    [1, "page", "oxplow:oxplow/commit?sha=abc1234", "abc1234", null, null],
    [2, "text", "oxplow:oxplow/file?path=a.rs", null, "fn main() {}", null],
    [3, "page", "oxplow:oxplow/page?page=tasks", "Tasks", null, "2026-10-24T10:00:00Z"],
  ];
  const opened: string[] = [];
  const view = render(<ContextDockBody streamId={null} onOpenPage={(r) => opened.push(r.id)} />);
  await waitFor(() => expect(view.getAllByTestId("context-panel-item").length).toBe(2));
  expect(view.getByText("abc1234")).toBeTruthy();
  expect(view.getByText("fn main() {}")).toBeTruthy();
  fireEvent.click(view.getAllByTestId("context-panel-remove")[0]!);
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.remove", { item: 1 }]));
  fireEvent.click(view.getByTestId("context-panel-clear"));
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.clear", { thread: "thr7" }]));
  fireEvent.click(view.getByTestId("context-panel-open-page"));
  expect(opened).toEqual(["oxplow:oxplow/page?page=context"]);
});

test("an empty basket says how things get in", async () => {
  setSelectedThread("thr7");
  rows = [];
  const view = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getByTestId("context-panel-empty").textContent).toBe("No context added yet"));
  expect(view.queryByTestId("context-panel-clear")).toBeNull();
});
