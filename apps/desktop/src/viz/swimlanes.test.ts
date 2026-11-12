import { afterEach, beforeEach, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// task-views' swimlanes (the `task.board` replacement): its bundle's own
// script, run against its own page with a stand-in for the client library's
// connection — the columns, dragging between and within them, renaming a
// card and opening one, as the frame shows them.

const dir = join(import.meta.dir, "../../../../examples/extensions/task-views/components/swimlanes");
const page = readFileSync(join(dir, "index.html"), "utf8").replace(/<script[\s\S]*?<\/script>/g, "");
const script = readFileSync(join(dir, "app.js"), "utf8");

type Item = { uri: string; title: string; stage: string };
// A list's own stages, in its order: not core's buckets, to show the lanes follow the list.
const STAGES: Array<[string, string, string]> = [
  ["backlog", "Backlog", "backlog"],
  ["ready", "Ready", "todo"],
  ["in_progress", "In Progress", "in_progress"],
  ["review", "In review", "in_progress"],
  ["blocked", "Blocked", "blocked"],
  ["done", "Done", "done"],
  ["canceled", "Canceled", "canceled"],
];
const ITEMS: Item[] = [
  { uri: "oxplow:oxplow/task?id=1&provider=a", title: "First", stage: "ready" },
  { uri: "oxplow:oxplow/task?id=2&provider=a", title: "Second", stage: "ready" },
  { uri: "oxplow:oxplow/task?id=3&provider=a", title: "Third", stage: "ready" },
  { uri: "oxplow:oxplow/task?id=4&provider=a", title: "Fourth", stage: "done" },
];
const run = (items: Item[]) => ({
  config: { id: "task_views/swimlanes" },
  params: { scope: "thread", thread_id: "t1" },
  result: { columns: ["uri", "title", "stage", "rank"], rows: items.map((i, n) => [i.uri, i.title, i.stage, n]), truncated: false },
});

let invokes: Array<[string, unknown]> = [];
let opened: Array<[string, unknown]> = [];
let rejection: unknown = null;

/** Load the page and start its script. */
async function start(items = ITEMS) {
  document.body.innerHTML = page;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (globalThis as any).oxplow = {
    connect: async () => ({
      data: {
        swimlanes: run(items).result,
        stages: {
          columns: ["value", "label", "bucket", "ord"],
          rows: STAGES.map(([value, label, bucket], n) => [value, label, bucket, n]),
          truncated: false,
        },
      },
      applyTheme() {},
      onUpdate: () => () => {},
      invoke: async (command: string, input: unknown) => {
        invokes.push([command, input]);
        if (rejection) throw rejection;
        return null;
      },
      invokePoint: async (point: string, context: unknown) => {
        invokes.push([point, context]);
        if (rejection) throw rejection;
        return null;
      },
      navigate: async (ref: string, opts: unknown) => {
        opened.push([ref, opts]);
        return null;
      },
    }),
  };
  new Function(script)();
  await new Promise((r) => setTimeout(r, 0));
}

const settle = () => new Promise((r) => setTimeout(r, 0));
const lane = (stage: string) => document.querySelector<HTMLElement>(`.lane[data-stage="${stage}"]`)!;
const card = (ref: string) => document.querySelector<HTMLElement>(`.card[data-ref="${ref}"]`)!;
const fire = (el: Element, type: string, init: Record<string, unknown> = {}) => {
  const e = new Event(type, { bubbles: true, cancelable: true });
  Object.assign(e, init);
  el.dispatchEvent(e);
};
const drag = async (ref: string, onto: Element) => {
  fire(card(ref), "dragstart");
  fire(onto, "dragover");
  fire(onto, "drop");
  fire(card(ref), "dragend");
  await settle();
};

beforeEach(() => {
  invokes = [];
  opened = [];
  rejection = null;
});
afterEach(() => {
  document.body.innerHTML = "";
});

test("one column per stage of the list, in its order and titled as it names them, each card in its stage's, with counts", async () => {
  await start();
  expect([...document.querySelectorAll(".lane")].map((l) => (l as HTMLElement).dataset.stage)).toEqual(
    STAGES.map(([id]) => id),
  );
  expect([...document.querySelectorAll(".lane h3")].map((h) => h.firstChild!.textContent!.trim())).toEqual(
    STAGES.map(([, title]) => title),
  );
  expect([...lane("ready").querySelectorAll(".card")].map((c) => (c as HTMLElement).dataset.ref)).toEqual([
    "oxplow:oxplow/task?id=1&provider=a",
    "oxplow:oxplow/task?id=2&provider=a",
    "oxplow:oxplow/task?id=3&provider=a",
  ]);
  expect(lane("ready").querySelector(".ox-badge")!.textContent).toBe("3");
  expect(lane("done").querySelectorAll(".card").length).toBe(1);
  expect(card("oxplow:oxplow/task?id=1&provider=a").classList.contains("ox-drag")).toBe(true);
});

test("a card dropped on another column is transitioned; the dragged-over column is marked", async () => {
  await start();
  fire(card("oxplow:oxplow/task?id=1&provider=a"), "dragstart");
  expect(card("oxplow:oxplow/task?id=1&provider=a").classList.contains("ox-dragging")).toBe(true);
  fire(lane("blocked"), "dragover");
  expect(lane("blocked").classList.contains("ox-drop-target")).toBe(true);
  fire(lane("blocked"), "drop");
  await settle();
  expect(lane("blocked").classList.contains("ox-drop-target")).toBe(false);
  expect(invokes).toEqual([["oxplow/task.move", { uri: "oxplow:oxplow/task?id=1&provider=a", stage: "blocked" }]]);
});

test("a drop within a column reorders it in its assignee's queue", async () => {
  await start();
  await drag("oxplow:oxplow/task?id=3&provider=a", card("oxplow:oxplow/task?id=1&provider=a"));
  await drag("oxplow:oxplow/task?id=1&provider=a", card("oxplow:oxplow/task?id=2&provider=a"));
  expect(invokes).toEqual([
    ["oxplow.tasks.reorder", { uri: "oxplow:oxplow/task?id=3&provider=a", before: "oxplow:oxplow/task?id=1&provider=a" }],
    ["oxplow.tasks.reorder", { uri: "oxplow:oxplow/task?id=1&provider=a", after: "oxplow:oxplow/task?id=2&provider=a" }],
  ]);

});

test("a title is edited in place: Enter submits it, Escape leaves it as it was", async () => {
  await start();
  const title = () => card("oxplow:oxplow/task?id=1&provider=a").querySelector<HTMLElement>(".title")!;
  title().click();
  const input = card("oxplow:oxplow/task?id=1&provider=a").querySelector<HTMLInputElement>("form .ox-input")!;
  expect(input.value).toBe("First");
  fire(input.closest("form")!, "keydown", { key: "Escape" });
  expect(title().textContent).toBe("First");
  expect(card("oxplow:oxplow/task?id=1&provider=a").querySelector("form")).toBe(null);

  title().click();
  const again = card("oxplow:oxplow/task?id=1&provider=a").querySelector<HTMLInputElement>("form .ox-input")!;
  again.value = "Renamed";
  const submit = new Event("submit", { bubbles: true, cancelable: true });
  again.closest("form")!.dispatchEvent(submit);
  await settle();
  expect(submit.defaultPrevented).toBe(true);
  expect(invokes).toEqual([["oxplow/task.edit", { uri: "oxplow:oxplow/task?id=1&provider=a", title: "Renamed" }]]);
  expect(opened).toEqual([]);
});

test("a rejection naming a field shows under the input", async () => {
  await start();
  card("oxplow:oxplow/task?id=2&provider=a").querySelector<HTMLElement>(".title")!.click();
  const form = card("oxplow:oxplow/task?id=2&provider=a").querySelector("form")!;
  form.querySelector<HTMLInputElement>(".ox-input")!.value = "";
  rejection = { code: "INVALID", message: "title can't be empty", field: "/title" };
  fire(form, "submit");
  await settle();
  const error = form.querySelector<HTMLElement>(".ox-error")!;
  expect(error.hidden).toBe(false);
  expect(error.textContent).toBe("title can't be empty");
  expect(card("oxplow:oxplow/task?id=2&provider=a").querySelector("form")).toBe(form);
});

test("a refusal of the board's commands is a one-line note above the board", async () => {
  await start();
  rejection = { code: "DENIED", message: "not deployed" };
  await drag("oxplow:oxplow/task?id=1&provider=a", lane("done"));
  const note = document.getElementById("note")!;
  expect(note.hidden).toBe(false);
  expect(note.textContent).toContain("not deployed");
  expect(note.textContent).toContain("deploy the extension");
});

test("clicking a card opens it; Cmd/Ctrl-click opens it in a new tab", async () => {
  await start();
  card("oxplow:oxplow/task?id=4&provider=a").click();
  fire(card("oxplow:oxplow/task?id=4&provider=a"), "click", { metaKey: true });
  fire(card("oxplow:oxplow/task?id=4&provider=a"), "click", { ctrlKey: true });
  await settle();
  expect(opened).toEqual([
    ["oxplow:oxplow/task?id=4&provider=a", undefined],
    ["oxplow:oxplow/task?id=4&provider=a", { newTab: true }],
    ["oxplow:oxplow/task?id=4&provider=a", { newTab: true }],
  ]);
});
