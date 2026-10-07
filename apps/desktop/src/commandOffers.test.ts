import { expect, test } from "bun:test";

import { bindInput, commandOffers, type OfferDeps } from "./commandOffers.js";
import type { CommandSpec, CommandUi } from "./tauri-bridge/generated/bindings.js";

// What search offers of the command bus: each command a person may run
// that says how (`ui`) and needs no ref, run the way its `ui` says.

function spec(id: string, ui: Partial<CommandUi>): CommandSpec {
  return {
    id,
    summary: "",
    input_schema: {},
    invokers: { human: true, agent: false, lens: false },
    confirm: "never",
    undoable: false,
    lifecycle: "stable",
    atomicity: "tx",
    effect: "write",
    needs: [],
    ui: { label: id, group: null, keywords: [], about: null, input: null, form: null, open_after: null, background: false, ...ui },
  } as unknown as CommandSpec;
}

function deps() {
  const calls: string[] = [];
  const d: OfferDeps = {
    openPage: (tabId) => calls.push(`open ${tabId}`),
    run: async (label, id, input) => {
      calls.push(`run ${id} ${JSON.stringify(input)}`);
      return { result: { id: 7 } } as never;
    },
    runInBackground: (label, id, input) => calls.push(`background ${id} ${JSON.stringify(input)}`),
  };
  return { d, calls };
}

const ctx = { streamId: "str2", threadId: "thr5" };

test("an input template binds the stream and thread; one that can't bind is unavailable", () => {
  expect(bindInput({ stream: "{{stream}}", thread: "{{thread}}", n: 3 }, ctx)).toEqual({
    input: { stream: "str2", thread: "thr5", n: 3 },
  });
  expect(bindInput(null, ctx)).toEqual({ input: {} });
  expect(bindInput({ stream: "{{stream}}" }, { streamId: null, threadId: null })).toBeNull();
  // A ref is bound where there's one; search has none.
  expect(bindInput({ ref: "{{ref}}" }, ctx)).toBeNull();
});

test("search offers ref-less commands under their group, by label and keywords", () => {
  const { d } = deps();
  const offers = commandOffers(
    [
      spec("oxplow.vcs.pull", { label: "Pull Changes", group: "Git", keywords: ["sync"] }),
      spec("oxplow.review.accept", { label: "Accept Review", about: "effort" }),
      spec("oxplow.vcs.push", { label: "Push Changes", input: { stream: "{{stream}}" } }),
    ],
    { streamId: null, threadId: null },
    d,
  );
  expect(offers.map((o) => [o.id, o.group, o.label])).toEqual([["oxplow.vcs.pull", "Git", "Pull Changes"]]);
  expect(offers[0].searchKey).toContain("sync");
});

test("a form opens its page; a background command runs in the background; another runs and opens what it made", async () => {
  const { d, calls } = deps();
  const offers = commandOffers(
    [
      spec("oxplow.work_item.create", { label: "New Task…", form: "page:new-task" }),
      spec("oxplow.vcs.pull", { label: "Pull Changes", input: { stream: "{{stream}}" }, background: true }),
      spec("oxplow.dashboard.create", {
        label: "New Dashboard…",
        input: { title: "Untitled dashboard" },
        open_after: "page:custom-dashboard?id={{result.id}}",
      }),
    ],
    ctx,
    d,
  );
  for (const o of offers) o.run();
  await new Promise((r) => setTimeout(r, 0));
  expect(calls).toEqual([
    "open page:new-task",
    'background oxplow.vcs.pull {"stream":"str2"}',
    'run oxplow.dashboard.create {"title":"Untitled dashboard"}',
    "open page:custom-dashboard?id=7",
  ]);
});
