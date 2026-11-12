import { expect, test } from "bun:test";
import { boardColumns, cardHint, cardMenu, choiceFields, dropAct, epicGroups, filtersOf, moreLabel, moreRef, priorityOf, records, stagesOf } from "../../../../extensions/oxplow-bundled/src/board/model.js";

// oxplow-bundled's Tasks board (a custom component, `src/board/`): the pure
// half its frame draws from — the columns from the list's stages, a card's
// menu from what the list can do, a drop as the command it runs, and the
// filters from the tab's view.

const item = (over: Record<string, unknown>) => ({
  uri: "oxplow:oxplow/task?id=tsk1&provider=oxtask", id: "tsk1", title: "Fix it", state: "todo", stage: "ready", native: '{"priority":"low","author":"user"}',
  is_open: 1, can_delete: 1, has_parent: 0, assignee: "oxplow:oxplow/thread?id=thr1",
  ...over,
});

test("rows read as records by column", () => {
  expect(records({ columns: ["a", "b"], rows: [[1, "x"]], truncated: false })).toEqual([{ a: 1, b: "x" }]);
  expect(records(undefined)).toEqual([]);
});

// oxplow's own list's stages, as the `task-stages` query reads them.
const stages = stagesOf([
  { value: "backlog", label: "Backlog", bucket: "backlog" },
  { value: "ready", label: "Ready", bucket: "todo" },
  { value: "in_progress", label: "In Progress", bucket: "in_progress" },
  { value: "blocked", label: "Blocked", bucket: "blocked" },
  { value: "done", label: "Done", bucket: "done" },
  { value: "canceled", label: "Canceled", bucket: "canceled" },
]);

test("the columns are the list's stages but its backlog and canceled ones; the agent's live card leads the first started one; Done says how many more it holds", () => {
  const cols = boardColumns(
    [item({ title: "A" }), item({ title: "B", state: "in_progress", stage: "in_progress" }), item({ uri: null, title: "Agent is working: x" }), item({ title: "C", state: "done", stage: "done" })],
    [{ stage: "ready", n: 1 }, { stage: "in_progress", n: 1 }, { stage: "done", n: 14 }],
    stages,
  );
  expect(cols.map((c) => c.stage)).toEqual(["ready", "in_progress", "blocked", "done"]);
  expect(cols.map((c) => c.label)).toEqual(["Ready", "In Progress", "Blocked", "Done"]);
  expect(cols.map((c) => c.bucket)).toEqual(["todo", "in_progress", "blocked", "done"]);
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

const assignees = [
  { uri: "oxplow:oxplow/thread?id=thr1", title: "Main" },
  { uri: "oxplow:oxplow/person", title: "You" },
];

test("a card's menu offers what its gates allow, each as the command it runs", () => {
  const fields = [{ name: "priority", title: "Priority", values: ["high", "low"] }];
  const menu = cardMenu(item({}), { fields, assignees, stages });
  const labels = menu.map((m) => (m.kind === "item" ? m.label : m.kind === "heading" ? `# ${m.label}` : "---"));
  // The list's other stages, each a move there.
  expect(labels).toEqual(["Open", "---", "Mark Backlog", "Mark In Progress", "Mark Blocked", "Mark Done", "Mark Canceled", "---", "# Priority", "High", "Low", "---", "# Assign to", "Main", "You", "Nobody", "---", "Delete"]);
  // Who it's for: its assignee checked, each choice an assign.
  expect(menu.find((m) => m.kind === "item" && m.label === "Mark Blocked")).toMatchObject({ id: "to-blocked", act: { command: "oxtask.task.move", input: { id: item({}).id, stage: "blocked" } } });
  expect(menu.find((m) => m.kind === "item" && m.label === "Main")).toMatchObject({ checked: true });
  expect(menu.find((m) => m.kind === "item" && m.label === "You")).toMatchObject({ checked: false, act: { command: "oxplow.tasks.assign", input: { uri: item({}).uri, to: "oxplow:oxplow/person" } } });
  expect(menu.find((m) => m.kind === "item" && m.label === "Nobody")).toMatchObject({ act: { command: "oxplow.tasks.assign", input: { to: null } } });
  const low = menu.find((m) => m.kind === "item" && m.label === "Low");
  expect(low).toMatchObject({ checked: true, act: { command: "oxtask.task.update", input: { id: item({}).id, priority: "low" } } });
  expect(menu.find((m) => m.kind === "item" && m.label === "Delete")).toMatchObject({ danger: true, act: { command: "oxtask.task.delete", input: { id: item({}).id } } });
  expect(menu[0]).toMatchObject({ kind: "item", label: "Open", open: item({}).uri });
  // An epic's child can leave it; a closed card is nobody's to assign.
  const child = cardMenu(item({ has_parent: 1, can_delete: 0 }), { fields: [], assignees, stages });
  expect(child.find((m) => m.kind === "item" && m.label === "Make top-level")).toMatchObject({ act: { command: "oxtask.task.update", input: { id: "tsk1", parent: null } } });
  const done = cardMenu(item({ state: "done", stage: "done", is_open: 0, assignee: null }), { fields: [], assignees, stages });
  expect(done.some((m) => m.kind === "heading" && m.label === "Assign to")).toBe(false);
});

test("the agent's live card has no menu and doesn't move", () => {
  const live = item({ uri: null, title: "Agent is working: x" });
  expect(cardMenu(live, { fields: [], assignees, stages })).toEqual([]);
  expect(dropAct(live, "done", null)).toBeNull();
});

test("a drop on another column moves the item to its stage; within its column it reorders in its queue", () => {
  const card = item({});
  expect(dropAct(card, "in_progress", null)).toEqual({ command: "oxtask.task.move", input: { id: card.id, stage: "in_progress" } });
  const other = item({ uri: "oxplow:oxplow/task?id=tsk2&provider=oxtask" });
  expect(dropAct(card, "ready", other)).toEqual({ command: "oxplow.tasks.reorder", input: { uri: card.uri, before: other.uri } });
  expect(dropAct(card, "ready", null)).toEqual({ command: "oxplow.tasks.reorder", input: { uri: card.uri } });
  expect(dropAct(card, "ready", card)).toBeNull();
});

test("the filters are the tab's view, else what the page ran with, else the defaults", () => {
  expect(filtersOf({ show: "pool" }, { show: "thread", priority: "high", q: null })).toEqual({ show: "pool", priority: "high", q: "" });
  expect(filtersOf(null, {})).toEqual({ show: "thread", priority: "any", q: "" });
});

test("Done's more opens Task List at the done items", () => {
  expect(moreRef()).toBe("oxplow:oxplow/page?page=task-list#state=done");
});

const EPIC = "oxplow:oxplow/task?id=tsk1&provider=oxtask";
const OTHER_EPIC = "oxplow:oxplow/task?id=tsk9&provider=oxtask";
const ref = (n: number) => `oxplow:oxplow/task?id=tsk${n}&provider=oxtask`;

test("To Do groups its cards under their epics, in order, the loose ones last; an epic in the column is its group's heading", () => {
  const all = [
    item({ uri: ref(2), title: "Loose", parent_uri: null }),
    item({ uri: EPIC, title: "An epic", parent_uri: null }),
    item({ uri: ref(3), title: "Child", parent_uri: EPIC, parent_title: "An epic" }),
    item({ uri: ref(4), title: "Other child", parent_uri: OTHER_EPIC, parent_title: "In progress epic" }),
    item({ uri: ref(5), title: "Child 2", parent_uri: EPIC, parent_title: "An epic" }),
    item({ uri: OTHER_EPIC, title: "In progress epic", state: "in_progress" }),
  ];
  const todo = all.filter((r) => r.state === "todo");
  const groups = epicGroups(todo, all);
  expect(groups.map((g) => [g.uri, g.title, g.epic?.title ?? null, g.cards.map((c) => c.title)])).toEqual([
    [EPIC, "An epic", "An epic", ["Child", "Child 2"]],
    [OTHER_EPIC, "In progress epic", null, ["Other child"]],
    [null, null, null, ["Loose"]],
  ]);
  // No epics at all: one group of every card, no heading.
  expect(epicGroups([item({ title: "A" })], [item({ title: "A" })]).map((g) => [g.uri, g.cards.length])).toEqual([[null, 1]]);
});

test("a compact card says the rest on hover: its epic, fields, thread and comments", () => {
  expect(cardHint(item({ title: "Fix it", parent_title: "An epic", fields: "high · user", assignee_shown: "Main", comments: 2 }))).toBe("Fix it\nin An epic\nhigh · user\nMain\n2 comments");
  expect(cardHint(item({ title: "Fix it", fields: null, comments: 0 }))).toBe("Fix it");
});

test("a card's priority is its list's priority field, for its coloured edge", () => {
  expect(priorityOf(item({ native: '{"priority":"urgent"}' }))).toBe("urgent");
  expect(priorityOf(item({ native: "{}" }))).toBeNull();
  expect(priorityOf(item({ native: '{"priority":"weird value"}' }))).toBeNull();
});

test("Done ends in a link to all of them in the Task List; another column with more says how many", () => {
  // Done by its bucket, whatever the list calls it.
  expect(moreLabel({ stage: "shipped", bucket: "done", label: "Shipped", cards: [], total: 465, more: 455 })).toBe("All 465 in Task List");
  expect(moreLabel({ stage: "ready", bucket: "todo", label: "Ready", cards: [], total: 12, more: 2 })).toBe("2 more…");
});

