import { expect, test } from "bun:test";
import { boardColumns, cardMenu, choiceFields, dropAct, filtersOf, moreRef, records } from "../../../../extensions/oxplow-bundled/src/board/model.js";

// oxplow-bundled's Tasks board (a custom component, `src/board/`): the pure
// half its frame draws from — the columns from the page's rows, a card's
// menu from what the list can do, a drop as the command it runs, and the
// filters from the tab's view.

const item = (over: Record<string, unknown>) => ({
  uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "Fix it", state: "todo", native: '{"priority":"low","author":"user"}',
  not_todo: 0, not_in_progress: 1, not_blocked: 1, not_done: 1, not_canceled: 1,
  can_order: 1, can_delete: 1, has_parent: 0, can_shelve: 1, can_bring: 0, thread_ref: "thr1",
  ...over,
});

test("rows read as records by column", () => {
  expect(records({ columns: ["a", "b"], rows: [[1, "x"]], truncated: false })).toEqual([{ a: 1, b: "x" }]);
  expect(records(undefined)).toEqual([]);
});

test("the board is four columns; the agent's live card leads In Progress; Done says how many more it holds", () => {
  const cols = boardColumns(
    [item({ title: "A" }), item({ title: "B", state: "in_progress" }), item({ uri: null, title: "Agent is working: x", state: "in_progress" }), item({ title: "C", state: "done" })],
    [{ state: "todo", n: 1 }, { state: "in_progress", n: 1 }, { state: "done", n: 14 }],
  );
  expect(cols.map((c) => c.state)).toEqual(["todo", "in_progress", "blocked", "done"]);
  expect(cols.map((c) => c.label)).toEqual(["To Do", "In Progress", "Blocked", "Done"]);
  expect(cols[1]!.cards.map((c) => c.title)).toEqual(["Agent is working: x", "B"]);
  expect(cols.map((c) => c.total)).toEqual([1, 1, 0, 14]);
  expect(cols[3]!.more).toBe(13);
  expect(cols[0]!.more).toBe(0);
});

test("only the list's editable choice fields are menu fields, with their values", () => {
  expect(choiceFields([
    { name: "priority", title: "Priority", kind: "enum", values: '["urgent","high","medium","low"]', read_only: 0 },
    { name: "author", title: "Filed by", kind: "enum", values: '["user","agent"]', read_only: 1 },
    { name: "estimate", title: "Estimate", kind: "number", values: null, read_only: 0 },
  ])).toEqual([{ name: "priority", title: "Priority", values: ["urgent", "high", "medium", "low"] }]);
});

test("a card's menu offers what its gates allow, each as the command it runs", () => {
  const fields = [{ name: "priority", title: "Priority", values: ["high", "low"] }];
  const menu = cardMenu(item({}), fields);
  const labels = menu.map((m) => (m.kind === "item" ? m.label : m.kind === "heading" ? `# ${m.label}` : "---"));
  expect(labels).toEqual(["Open", "---", "Mark In Progress", "Mark Blocked", "Mark Done", "Mark Canceled", "---", "# Priority", "High", "Low", "---", "Send to backlog", "Delete"]);
  const low = menu.find((m) => m.kind === "item" && m.label === "Low");
  expect(low).toMatchObject({ checked: true, act: { command: "oxplow.work_item.update", input: { uri: item({}).uri, native: { priority: "low" } } } });
  expect(menu.find((m) => m.kind === "item" && m.label === "Delete")).toMatchObject({ danger: true, act: { command: "oxplow.work_item.delete", input: { uri: item({}).uri } } });
  expect(menu[0]).toMatchObject({ kind: "item", label: "Open", open: item({}).uri });
  // An epic's child can leave it; a card on another list can come here.
  const child = cardMenu(item({ has_parent: 1, can_shelve: 0, can_bring: 1, can_delete: 0 }), []);
  expect(child.filter((m) => m.kind === "item").map((m) => m.kind === "item" && m.label)).toContain("Make top-level");
  expect(child.find((m) => m.kind === "item" && m.label === "Bring to this thread")).toMatchObject({ act: { command: "oxplow.work_item.move", input: { to: { thread: "thr1" } } } });
  expect(child.find((m) => m.kind === "item" && m.label === "Make top-level")).toMatchObject({ act: { command: "oxplow.work_item.update", input: { parent_uri: "" } } });
});

test("the agent's live card has no menu and doesn't move", () => {
  const live = item({ uri: null, title: "Agent is working: x", state: "in_progress" });
  expect(cardMenu(live, [])).toEqual([]);
  expect(dropAct(live, "done", null)).toBeNull();
});

test("a drop on another column moves the item there; within its column it reorders when the list can", () => {
  const card = item({});
  expect(dropAct(card, "in_progress", null)).toEqual({ command: "oxplow.work_item.transition", input: { uri: card.uri, to: "in_progress" } });
  const other = item({ uri: "oxplow:oxplow/work_item?id=tsk2&provider=oxplow" });
  expect(dropAct(card, "todo", other)).toEqual({ command: "oxplow.work_item.reorder", input: { uri: card.uri, before: other.uri } });
  expect(dropAct(card, "todo", null)).toEqual({ command: "oxplow.work_item.reorder", input: { uri: card.uri } });
  expect(dropAct(item({ can_order: 0 }), "todo", other)).toBeNull();
  expect(dropAct(card, "todo", card)).toBeNull();
});

test("the filters are the tab's view, else what the page ran with, else the defaults", () => {
  expect(filtersOf({ show: "backlog" }, { show: "thread", priority: "high", q: null })).toEqual({ show: "backlog", priority: "high", q: "" });
  expect(filtersOf(null, {})).toEqual({ show: "thread", priority: "any", q: "" });
});

test("Done's more opens Task List at the closed items of the same scope", () => {
  expect(moreRef({ show: "stream", priority: "any", q: "" })).toBe("oxplow:oxplow/page?page=task-list#show=stream&state=done");
});
