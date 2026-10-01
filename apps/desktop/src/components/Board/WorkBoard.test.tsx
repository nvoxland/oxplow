import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { WORK_ITEM_DRAG_MIME } from "../../dragMimes.js";

// The Board (P6.E1a): work items by canonical state; a card moves by
// drag or by its right-click menu, through work_item.transition.

const realApi = await import("../../api.js");
const realQuerySql = realApi.querySql;
const realRunCommand = realApi.runCommand;
const ran: Array<[string, unknown]> = [];
mock.module("../../api.js", () => ({
  ...realApi,
  querySql: async (sql: string, ...rest: unknown[]) => {
    if (!sql.includes("FROM v_work_item w")) return (realQuerySql as (...a: unknown[]) => unknown)(sql, ...rest);
    return {
      columns: ["ref", "provider", "title", "body", "state", "native_state", "parent_ref", "created_at", "updated_at", "task_id", "thread_id", "status", "priority", "sort_index", "author", "completed_at", "note_count"],
      rows: [
        ["work_item:oxplow:tsk1", "oxplow", "Plan it", "", "todo", "ready", null, "t", "t", 1, 1, "ready", "medium", 0, "user", null, 0],
        ["work_item:oxplow:tsk2", "oxplow", "Ship it", "", "in_progress", "in_progress", null, "t", "t", 2, 1, "in_progress", "high", 1, "agent", null, 2],
        ["work_item:fake:W-1", "fake", "Their bug", "", "todo", "Backlog", null, "t", "t", null, null, null, null, null, null, null, 0],
      ],
      truncated: false,
      reads: { models: ["v_work_item"], tables: [], measures: [] },
      freshness: {},
    };
  },
  runCommand: async (name: string, input: unknown, ...rest: unknown[]) => {
    if (!name.startsWith("work_item.") && !name.startsWith("fake.")) return (realRunCommand as (...a: unknown[]) => unknown)(name, input, ...rest);
    ran.push([name, input]);
    return { result: null, audit_id: 1, event_id: null, undo: null };
  },
}));
const { WorkBoard } = await import("./WorkBoard.js");

afterEach(() => {
  ran.length = 0;
  cleanup();
});

test("cards sit in their state's column", async () => {
  const view = render(<WorkBoard scope="all" onOpenPage={() => {}} />);
  const todo = await waitFor(() => view.getByTestId("board-column-todo"));
  expect(todo.textContent).toContain("Plan it");
  expect(view.getByTestId("board-column-in_progress").textContent).toContain("Ship it");
  expect(view.getByTestId("board-column-done").textContent).not.toContain("Plan it");
});

test("dropping a card on a column, or its menu's Move To, transitions it", async () => {
  const view = render(<WorkBoard scope="all" onOpenPage={() => {}} />);
  await waitFor(() => view.getByText("Plan it"));
  const data = new Map<string, string>([[WORK_ITEM_DRAG_MIME, "work_item:oxplow:tsk1"]]);
  const dataTransfer = { getData: (k: string) => data.get(k) ?? "", types: [WORK_ITEM_DRAG_MIME], dropEffect: "move" };
  fireEvent.dragOver(view.getByTestId("board-column-blocked"), { dataTransfer });
  fireEvent.drop(view.getByTestId("board-column-blocked"), { dataTransfer });
  await waitFor(() => expect(ran).toEqual([["work_item.transition", { ref: "work_item:oxplow:tsk1", to: "blocked" }]]));

  fireEvent.contextMenu(view.getByText("Ship it"));
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-board-move-done")));
  await waitFor(() => expect(ran[1]).toEqual(["work_item.transition", { ref: "work_item:oxplow:tsk2", to: "done" }]));
});

// P6b.C3: every provider's card opens its page and moves through its own
// provider (`<provider>.transition` with the canonical state).
test("another provider's card links to its page and moves through its provider", async () => {
  const opened: string[] = [];
  const view = render(<WorkBoard scope="all" onOpenPage={(ref) => opened.push(ref.id)} />);
  fireEvent.click(await waitFor(() => view.getByText("Their bug")));
  expect(opened).toEqual(["work_item:fake:W-1"]);
  const data = new Map<string, string>([[WORK_ITEM_DRAG_MIME, "work_item:fake:W-1"]]);
  const dataTransfer = { getData: (k: string) => data.get(k) ?? "", types: [WORK_ITEM_DRAG_MIME], dropEffect: "move" };
  fireEvent.drop(view.getByTestId("board-column-done"), { dataTransfer });
  await waitFor(() => expect(ran).toEqual([["fake.transition", { ref: "work_item:fake:W-1", to: "done" }]]));
});
