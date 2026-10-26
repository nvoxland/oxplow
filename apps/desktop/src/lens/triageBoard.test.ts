import { afterEach, beforeEach, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// The triage example's TODO board: its bundle's own script against its own
// page, with a stand-in for the client library. Each card has a form to
// decide what to do about the TODO; it runs the extension's own command
// and reads the decisions back. While its commands are refused (the
// extension isn't deployed, or is disabled), it shows the TODOs without
// decisions, and says why.

const dir = join(import.meta.dir, "../../../../examples/extensions/triage/components/todo-board");
const page = readFileSync(join(dir, "index.html"), "utf8").replace(/<script[\s\S]*?<\/script>/g, "");
const script = readFileSync(join(dir, "app.js"), "utf8");

const run = {
  config: { id: "triage/todo-board" },
  params: {},
  result: { columns: ["path", "line", "kind"], rows: [["src/lib.rs", 1, "bug"]], truncated: false },
};

let decisions: Record<string, string> = {};
let allowed = true;
let invoked: Array<[string, unknown]> = [];
let refuseDecide: { code: string; message: string; field?: string } | null = null;

async function start() {
  document.body.innerHTML = page;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (globalThis as any).oxplow = {
    connect: async () => ({
      run,
      applyTheme() {},
      onUpdate() {
        return () => {};
      },
      navigate: async () => null,
      invoke: async (command: string, input: { path: string; line: number; decision: string }) => {
        invoked.push([command, input]);
        if (!allowed) throw { code: "DENIED", message: "not deployed" };
        if (command === "triage.todos.decide") {
          if (refuseDecide) throw refuseDecide;
          decisions[`${input.path}:${input.line}`] = input.decision;
          return { result: input, children: [] };
        }
        const todos = run.result.rows.map(([path, line, kind]) => ({
          path,
          line,
          kind,
          decision: decisions[`${path}:${line}`] ?? null,
        }));
        return { result: { todos, decisions }, children: [] };
      },
    }),
  };
  new Function(script)();
  await settle();
}

const settle = () => new Promise((r) => setTimeout(r, 0));
const select = () => document.querySelector<HTMLSelectElement>("form.decide select")!;
const form = () => document.querySelector<HTMLFormElement>("form.decide")!;

beforeEach(() => {
  decisions = {};
  allowed = true;
  invoked = [];
  refuseDecide = null;
});
afterEach(() => {
  document.body.innerHTML = "";
});

test("a card's form records a decision through the extension's command and reads it back", async () => {
  await start();
  expect(select().value).toBe("");
  select().value = "fix-now";
  form().dispatchEvent(new Event("submit", { cancelable: true }));
  // It decides, then reads the decisions again and redraws from them.
  const reads = () => invoked.filter(([c]) => c === "triage.todos.decisions").length;
  for (let i = 0; i < 20 && reads() < 2; i++) await settle();
  await settle();
  expect(invoked).toContainEqual(["triage.todos.decide", { path: "src/lib.rs", line: 1, decision: "fix-now" }]);
  expect(reads()).toBe(2);
  // The redrawn card's select comes from what was read back.
  expect(select().value).toBe("fix-now");
  expect(document.querySelectorAll("form.decide")).toHaveLength(1);
});

test("a refused decision shows under the form, and an empty one is never sent", async () => {
  await start();
  form().dispatchEvent(new Event("submit", { cancelable: true }));
  await settle();
  expect(form().querySelector(".ox-error")!.textContent).toContain("Pick a decision");
  expect(invoked.filter(([c]) => c === "triage.todos.decide")).toEqual([]);

  refuseDecide = { code: "INVALID", message: "decision must be one of keep, fix-now, wontfix", field: "/decision" };
  select().value = "keep";
  form().dispatchEvent(new Event("submit", { cancelable: true }));
  await settle();
  expect(form().querySelector(".ox-error")!.textContent).toContain("must be one of");
});

test("while its commands are refused it shows the TODOs, can't decide, and says why", async () => {
  allowed = false;
  await start();
  expect(document.querySelectorAll(".card")).toHaveLength(1);
  expect(select().disabled).toBe(true);
  expect(document.getElementById("status")!.textContent).toContain("Settings → Extensions");
});
