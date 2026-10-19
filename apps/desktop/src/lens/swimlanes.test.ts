import { afterEach, beforeEach, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// task-views' swimlanes (the `work_item.board` replacement): its bundle's own
// script, run against its own page with a stand-in for the client library's
// connection — the columns, dragging between and within them, renaming a
// card and opening one, as the frame shows them.

const dir = join(import.meta.dir, "../../../../examples/extensions/task-views/components/swimlanes");
const page = readFileSync(join(dir, "index.html"), "utf8").replace(/<script[\s\S]*?<\/script>/g, "");
const script = readFileSync(join(dir, "app.js"), "utf8");

type Item = { ref: string; title: string; state: string };
const ITEMS: Item[] = [
  { ref: "work_item:a:1", title: "First", state: "todo" },
  { ref: "work_item:a:2", title: "Second", state: "todo" },
  { ref: "work_item:a:3", title: "Third", state: "todo" },
  { ref: "work_item:a:4", title: "Fourth", state: "done" },
];
const run = (items: Item[]) => ({
  lens: { id: "task-views/swimlanes" },
  params: { scope: "thread", thread_id: "t1" },
  result: { columns: ["ref", "title", "state", "rank"], rows: items.map((i, n) => [i.ref, i.title, i.state, n]), truncated: false },
});

let invokes: Array<[string, unknown]> = [];
let opened: Array<[string, unknown]> = [];
let queries: string[] = [];
let rejection: unknown = null;

/** Load the page and start its script; `features` is what the work list says it can do. */
async function start(features: string[], items = ITEMS) {
  document.body.innerHTML = page;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (globalThis as any).oxplow = {
    connect: async () => ({
      run: run(items),
      applyTheme() {},
      onUpdate: () => () => {},
      query: async (asset: string) => {
        queries.push(asset);
        if (asset === "features") return { result: { columns: ["features"], rows: [[JSON.stringify(features)]], truncated: false } };
        return run(items);
      },
      invoke: async (command: string, input: unknown) => {
        invokes.push([command, input]);
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
const lane = (state: string) => document.querySelector<HTMLElement>(`.lane[data-state="${state}"]`)!;
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
  queries = [];
  rejection = null;
});
afterEach(() => {
  document.body.innerHTML = "";
});

test("one column per state, each card in its state's, with counts", async () => {
  await start([]);
  expect([...document.querySelectorAll(".lane")].map((l) => (l as HTMLElement).dataset.state)).toEqual([
    "todo",
    "in_progress",
    "blocked",
    "done",
    "canceled",
  ]);
  expect([...lane("todo").querySelectorAll(".card")].map((c) => (c as HTMLElement).dataset.ref)).toEqual([
    "work_item:a:1",
    "work_item:a:2",
    "work_item:a:3",
  ]);
  expect(lane("todo").querySelector(".ox-badge")!.textContent).toBe("3");
  expect(lane("done").querySelectorAll(".card").length).toBe(1);
  expect(card("work_item:a:1").classList.contains("ox-drag")).toBe(true);
});

test("a card dropped on another column is transitioned; the dragged-over column is marked", async () => {
  await start([]);
  fire(card("work_item:a:1"), "dragstart");
  expect(card("work_item:a:1").classList.contains("ox-dragging")).toBe(true);
  fire(lane("blocked"), "dragover");
  expect(lane("blocked").classList.contains("ox-drop-target")).toBe(true);
  fire(lane("blocked"), "drop");
  await settle();
  expect(lane("blocked").classList.contains("ox-drop-target")).toBe(false);
  expect(invokes).toEqual([["oxplow.work_item.transition", { ref: "work_item:a:1", to: "blocked" }]]);
});

test("a drop within a column reorders it only when the work list keeps an order", async () => {
  await start(["ordering"]);
  await drag("work_item:a:3", card("work_item:a:1"));
  await drag("work_item:a:1", card("work_item:a:2"));
  expect(invokes).toEqual([
    ["oxplow.work_item.reorder", { ref: "work_item:a:3", before: "work_item:a:1" }],
    ["oxplow.work_item.reorder", { ref: "work_item:a:1", after: "work_item:a:2" }],
  ]);

  invokes = [];
  await start([]);
  await drag("work_item:a:3", card("work_item:a:1"));
  expect(invokes).toEqual([]);
});

test("a title is edited in place: Enter submits it, Escape leaves it as it was", async () => {
  await start([]);
  const title = () => card("work_item:a:1").querySelector<HTMLElement>(".title")!;
  title().click();
  const input = card("work_item:a:1").querySelector<HTMLInputElement>("form .ox-input")!;
  expect(input.value).toBe("First");
  fire(input.closest("form")!, "keydown", { key: "Escape" });
  expect(title().textContent).toBe("First");
  expect(card("work_item:a:1").querySelector("form")).toBe(null);

  title().click();
  const again = card("work_item:a:1").querySelector<HTMLInputElement>("form .ox-input")!;
  again.value = "Renamed";
  const submit = new Event("submit", { bubbles: true, cancelable: true });
  again.closest("form")!.dispatchEvent(submit);
  await settle();
  expect(submit.defaultPrevented).toBe(true);
  expect(invokes).toEqual([["oxplow.work_item.update", { ref: "work_item:a:1", title: "Renamed" }]]);
  expect(opened).toEqual([]);
});

test("a rejection naming a field shows under the input", async () => {
  await start([]);
  card("work_item:a:2").querySelector<HTMLElement>(".title")!.click();
  const form = card("work_item:a:2").querySelector("form")!;
  form.querySelector<HTMLInputElement>(".ox-input")!.value = "";
  rejection = { code: "INVALID", message: "title can't be empty", field: "/title" };
  fire(form, "submit");
  await settle();
  const error = form.querySelector<HTMLElement>(".ox-error")!;
  expect(error.hidden).toBe(false);
  expect(error.textContent).toBe("title can't be empty");
  expect(card("work_item:a:2").querySelector("form")).toBe(form);
});

test("a refusal of the board's commands is a one-line note above the board", async () => {
  await start([]);
  rejection = { code: "DENIED", message: "not deployed" };
  await drag("work_item:a:1", lane("done"));
  const note = document.getElementById("note")!;
  expect(note.hidden).toBe(false);
  expect(note.textContent).toContain("not deployed");
  expect(note.textContent).toContain("deploy the extension");
});

test("clicking a card opens it; Cmd/Ctrl-click opens it in a new tab", async () => {
  await start([]);
  card("work_item:a:4").click();
  fire(card("work_item:a:4"), "click", { metaKey: true });
  fire(card("work_item:a:4"), "click", { ctrlKey: true });
  await settle();
  expect(opened).toEqual([
    ["work_item:a:4", undefined],
    ["work_item:a:4", { newTab: true }],
    ["work_item:a:4", { newTab: true }],
  ]);
});
