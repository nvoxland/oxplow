import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// P6b.C3: another provider's work item has a page of its own, and what it
// offers follows the provider's declared features.

const realApi = await import("../api.js");
const realQuerySql = realApi.querySql;
const realRunCommand = realApi.runCommand;
let features: Record<string, boolean> = {};
const ran: Array<[string, unknown]> = [];
const lensRuns: Array<[string, unknown]> = [];
mock.module("../api.js", () => ({
  ...realApi,
  querySql: async (sql: string, ...rest: unknown[]) => {
    if (sql.includes("FROM v_capability_provider")) {
      return {
        columns: ["capability", "provider", "extension", "features", "active"],
        rows: [["work_items", "fake", "tracker", JSON.stringify(features), 1]],
        truncated: false,
        reads: { models: ["v_capability_provider"], tables: [], measures: [] },
        freshness: {},
      };
    }
    if (sql.includes("FROM v_work_item w")) {
      return {
        columns: ["ref", "provider", "title", "body", "state", "native_state", "parent_ref", "created_at", "updated_at", "task_id", "thread_id", "status", "priority", "sort_index", "author", "completed_at", "note_count"],
        rows: [["work_item:fake:W-1", "fake", "Their bug", "It breaks.", "todo", "Backlog", "work_item:fake:W-0", "t", "t", null, null, null, null, null, null, null, 0]],
        truncated: false,
        reads: { models: ["v_work_item"], tables: [], measures: [] },
        freshness: {},
      };
    }
    return (realQuerySql as (...a: unknown[]) => unknown)(sql, ...rest);
  },
  runCommand: async (name: string, input: unknown, ...rest: unknown[]) => {
    if (!name.startsWith("fake.")) return (realRunCommand as (...a: unknown[]) => unknown)(name, input, ...rest);
    ran.push([name, input]);
    return { result: null, audit_id: 1, event_id: null, inverse: null };
  },
  listExtensions: async () => [
    {
      name: "x",
      enabled: true,
      ui: {
        slots: [
          { slot: "work_item.detail.body", lensId: "x/body" },
          { slot: "work_item.detail.sidebar", lensId: "x/side" },
        ],
        commands: [],
      },
      lenses: ["x/body", "x/side"].map((id) => ({ id, params: [{ name: "ref", label: null, default: null }] })),
    },
  ],
  runLens: async (id: string, params: unknown) => {
    lensRuns.push([id, params]);
    return { lens: { id, title: id, columns: [], viz: "table", actions: [] }, params, result: { columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] } }, alert: null };
  },
}));
const { WorkItemPage } = await import("./WorkItemPage.js");

afterEach(() => {
  ran.length = 0;
  lensRuns.length = 0;
  cleanup();
});

const page = () => render(<WorkItemPage workItemRef="work_item:fake:W-1" streamId={null} onOpenPage={() => {}} />);

test("a provider without comments, links or hierarchy offers none of them", async () => {
  features = {};
  const view = page();
  await waitFor(() => expect(view.getByTestId("work-item-page").textContent).toContain("Their bug"));
  expect(view.getByTestId("work-item-page").textContent).toContain("It breaks.");
  expect(view.queryByTestId("work-item-comment-open")).toBeNull();
  expect(view.queryByTestId("work-item-link-open")).toBeNull();
  expect(view.queryByTestId("work-item-parent")).toBeNull();
});

test("comments, links and a parent show when the provider declares them; Comment runs its command", async () => {
  features = { comments: true, links: true, hierarchy: true };
  const view = page();
  fireEvent.click(await waitFor(() => view.getByTestId("work-item-comment-open")));
  expect(view.getByTestId("work-item-link-open")).toBeTruthy();
  expect(view.getByTestId("work-item-parent").textContent).toContain("work_item:fake:W-0");
  fireEvent.change(view.getByTestId("work-item-comment-input"), { target: { value: "Seen it too." } });
  fireEvent.click(view.getByTestId("work-item-comment-submit"));
  await waitFor(() =>
    expect(ran).toEqual([["fake.comment", { ref: "work_item:fake:W-1", body: "Seen it too." }]]),
  );
});

test("Move To runs the provider's transition; the item's slots get its ref", async () => {
  features = {};
  const view = page();
  fireEvent.click(await waitFor(() => view.getByTestId("work-item-move-done")));
  await waitFor(() => expect(ran).toEqual([["fake.transition", { ref: "work_item:fake:W-1", to: "done" }]]));
  await waitFor(() =>
    expect(lensRuns.map(([id, p]) => [id, p])).toEqual(
      expect.arrayContaining([
        ["x/body", { ref: "work_item:fake:W-1" }],
        ["x/side", { ref: "work_item:fake:W-1" }],
      ]),
    ),
  );
});
