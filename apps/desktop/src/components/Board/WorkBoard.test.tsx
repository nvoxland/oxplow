import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { WORK_ITEM_DRAG_MIME } from "../../dragMimes.js";
import { personSpec, recordRefOffers } from "../refCommandsTestSupport.js";

// The Board (P6.E1a): work items by canonical state; a card moves by
// drag or by its right-click menu, through oxplow.work_item.transition.

const realApi = await import("../../api.js");
const realQuerySql = realApi.querySql;
const realRunCommand = realApi.runCommand;
const ran: Array<[string, unknown]> = [];
let extensions: unknown[] = [];
mock.module("../../api.js", () => ({
  ...realApi,
  listExtensions: async () => extensions,
  listPersonCommands: async () => [
    personSpec("tracker.item.estimate", { label: "Estimate in Fake", group: "Tracker", about: "work_item", input: { ref: "{{ref}}" } }),
    personSpec("tracker.commit.sync", { label: "Sync", group: "Tracker", about: "commit" }),
  ],
  querySql: async (sql: string, ...rest: unknown[]) => {
    if (!sql.includes("FROM v_work_item w")) return (realQuerySql as (...a: unknown[]) => unknown)(sql, ...rest);
    return {
      columns: ["ref", "provider", "title", "body", "state", "parent_ref", "thread_id", "rank", "closed_at", "created_at", "updated_at", "native", "comment_count"],
      rows: [
        ["work_item:oxplow:tsk1", "oxplow", "Plan it", "", "todo", null, 1, 0, null, "t", "t", '{"priority":"medium"}', 0],
        ["work_item:oxplow:tsk2", "oxplow", "Ship it", "", "in_progress", null, 1, 1, null, "t", "t", '{"priority":"high"}', 2],
        ["work_item:fake:W-1", "fake", "Their bug", "", "todo", null, null, null, null, "t", "t", null, 0],
      ],
      truncated: false,
      reads: { models: ["v_work_item"], tables: [], measures: [] },
      freshness: {},
    };
  },
  runCommand: async (name: string, input: unknown, ...rest: unknown[]) => {
    if (!name.startsWith("oxplow.work_item.") && !name.startsWith("fake.")) return (realRunCommand as (...a: unknown[]) => unknown)(name, input, ...rest);
    ran.push([name, input]);
    return { result: null, audit_id: 1, event_id: null, undo: null };
  },
}));
const { WorkBoard } = await import("./WorkBoard.js");

/** A work-item drag carrying one card. */
const drag = (ref: string) => JSON.stringify({ refs: [ref], items: [], fromThreadId: null });

afterEach(() => {
  extensions = [];
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
  const data = new Map<string, string>([[WORK_ITEM_DRAG_MIME, drag("work_item:oxplow:tsk1")]]);
  const dataTransfer = { getData: (k: string) => data.get(k) ?? "", types: [WORK_ITEM_DRAG_MIME], dropEffect: "move" };
  fireEvent.dragOver(view.getByTestId("board-column-blocked"), { dataTransfer });
  fireEvent.drop(view.getByTestId("board-column-blocked"), { dataTransfer });
  await waitFor(() => expect(ran).toEqual([["oxplow.work_item.transition", { ref: "work_item:oxplow:tsk1", to: "blocked" }]]));

  fireEvent.contextMenu(view.getByText("Ship it"));
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-board-move-done")));
  await waitFor(() => expect(ran[1]).toEqual(["oxplow.work_item.transition", { ref: "work_item:oxplow:tsk2", to: "done" }]));
});

// P6b.C3, P7.A1: every provider's card opens its page and moves through
// `oxplow.work_item.transition` with the canonical state — the bus dispatches it.
test("another provider's card links to its page and moves through oxplow.work_item.transition", async () => {
  const opened: string[] = [];
  const view = render(<WorkBoard scope="all" onOpenPage={(ref) => opened.push(ref.id)} />);
  fireEvent.click(await waitFor(() => view.getByText("Their bug")));
  expect(opened).toEqual(["work_item:fake:W-1"]);
  const data = new Map<string, string>([[WORK_ITEM_DRAG_MIME, drag("work_item:fake:W-1")]]);
  const dataTransfer = { getData: (k: string) => data.get(k) ?? "", types: [WORK_ITEM_DRAG_MIME], dropEffect: "move" };
  fireEvent.drop(view.getByTestId("board-column-done"), { dataTransfer });
  await waitFor(() => expect(ran).toEqual([["oxplow.work_item.transition", { ref: "work_item:fake:W-1", to: "done" }]]));
});

// The commands about work items (their `ui.about`) join a card's
// right-click menu, bound to that card's ref.
test("a card's menu offers the commands about its item", async () => {
  recordRefOffers(ran);
  const view = render(<WorkBoard scope="all" onOpenPage={() => {}} />);
  await waitFor(() => view.getByText("Their bug"));
  await new Promise((r) => setTimeout(r, 0));
  fireEvent.contextMenu(view.getByText("Their bug"));
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-ref-commands-Tracker")));
  const item = await waitFor(() => view.getByTestId("menu-item-ref-command-tracker.item.estimate"));
  expect(item.textContent).toContain("Estimate in Fake");
  expect(view.queryByTestId("menu-item-ref-command-tracker.commit.sync")).toBeNull();
  fireEvent.click(item);
  await waitFor(() => expect(ran).toEqual([["tracker.item.estimate", { ref: "work_item:fake:W-1" }]]));
});

// A right-click anywhere on a card — its title link too — opens the card's
// menu and nothing else; opening the item in a new tab is in that menu.
test("right-clicking a card's title opens the card's menu, not a new tab", async () => {
  const opened: string[] = [];
  const view = render(<WorkBoard scope="all" onOpenPage={(ref) => opened.push(ref.id)} />);
  fireEvent.contextMenu(await waitFor(() => view.getByText("Ship it")));
  await waitFor(() => view.getByTestId("menu-item-board-move-done"));
  expect(opened).toEqual([]);
  fireEvent.click(view.getByTestId("menu-item-board-open-new-tab"));
  expect(opened.length).toBe(1);
});
