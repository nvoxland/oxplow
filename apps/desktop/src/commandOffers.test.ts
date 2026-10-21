import { expect, test } from "bun:test";

import { bindInput, commandOffers, refOffers, type OfferDeps } from "./commandOffers.js";
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
    access: "write",
    needs: [],
    ui: { label: id, group: null, keywords: [], about: null, input: null, form: null, open_after: null, background: false, shortcut: null, while_typing: false, menu: null, when: null, ...ui },
  } as unknown as CommandSpec;
}

function deps() {
  const calls: string[] = [];
  const d: OfferDeps = {
    openPage: (tabId) => calls.push(`open ${tabId}`),
    openForm: (name) => calls.push(`form ${name}`),
    run: async (label, id, input) => {
      calls.push(`run ${id} ${JSON.stringify(input)}`);
      return { result: { id: 7 } } as never;
    },
    runInBackground: (label, id, input) => calls.push(`background ${id} ${JSON.stringify(input)}`),
    // The server binds `open_after` from the result (`command_open_after`).
    openAfter: async (id, result) => {
      calls.push(`open_after ${id} ${JSON.stringify(result)}`);
      return "oxplow:oxplow/page?id=7&page=custom-dashboard";
    },
  };
  return { d, calls };
}

const ctx = { streamId: "str2", threadId: "thr5" };

test("an input template binds the stream and thread; one that can't bind is unavailable", () => {
  expect(bindInput({ stream: "{{stream}}", thread: "{{thread}}", n: 3 }, ctx)).toEqual({
    input: { stream: "oxplow:oxplow/stream?id=str2", thread: "oxplow:oxplow/thread?id=thr5", n: 3 },
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
      spec("oxplow.review.accept", { label: "Accept Review", about: "oxplow/effort" }),
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
      spec("oxplow.work_item.create", { label: "New Task…", form: "oxplow:oxplow/page?page=new-task" }),
      spec("oxplow.vcs.pull", { label: "Pull Changes", input: { stream: "{{stream}}" }, background: true }),
      spec("oxplow.dashboard.create", {
        label: "New Dashboard…",
        input: { title: "Untitled dashboard" },
        open_after: "oxplow:oxplow/page?id=%7B%7Bresult.id%7D%7D&page=custom-dashboard",
      }),
    ],
    ctx,
    d,
  );
  for (const o of offers) o.run();
  await new Promise((r) => setTimeout(r, 0));
  expect(calls).toEqual([
    "open oxplow:oxplow/page?page=new-task",
    'background oxplow.vcs.pull {"stream":"oxplow:oxplow/stream?id=str2"}',
    'run oxplow.dashboard.create {"title":"Untitled dashboard"}',
    'open_after oxplow.dashboard.create {"id":7}',
    "open oxplow:oxplow/page?id=7&page=custom-dashboard",
  ]);
});

test("a form that isn't a tab id is one of the window's own; an offer says its shortcut, menu place and whether its `when` holds", () => {
  const { d, calls } = deps();
  const offers = commandOffers(
    [
      spec("oxplow.thread.create", { label: "New Thread…", form: "new-thread", when: "streamShown" }),
      spec("oxplow.editor.save", { label: "Save", shortcut: "Ctrl/Cmd+S", while_typing: true, menu: { bar: "file", order: 40 }, when: "fileShown && fileDirty" }),
    ],
    { ...ctx, when: { streamShown: true, fileShown: true, fileDirty: false } },
    d,
  );
  offers[0].run();
  expect(calls).toEqual(["form new-thread"]);
  expect(offers.map((o) => [o.id, o.shortcut, o.whileTyping, o.menu, o.enabled])).toEqual([
    ["oxplow.thread.create", undefined, false, null, true],
    ["oxplow.editor.save", "Ctrl/Cmd+S", true, { bar: "file", order: 40 }, false],
  ]);
});

test("a ref's menus offer the commands about its kind, the ref bound into their input", async () => {
  const { d, calls } = deps();
  const specs = [
    spec("oxplow.review.accept", { label: "Accept Review", about: "oxplow/effort", input: { ref: "{{ref}}" } }),
    spec("tracker.item.estimate", { label: "Estimate", about: "oxplow/work_item", input: { ref: "{{ref}}", points: 3 } }),
    spec("tracker.item.flag", { label: "Flag", about: "oxplow/work_item" }),
    spec("oxplow.vcs.pull", { label: "Pull" }),
  ];
  const offers = refOffers(specs, "oxplow:oxplow/work_item?id=W-1&provider=fake", ctx, d);
  expect(offers.map((o) => o.id)).toEqual(["tracker.item.estimate", "tracker.item.flag"]);
  for (const o of offers) o.run();
  await new Promise((r) => setTimeout(r, 0));
  expect(calls).toEqual([
    'run tracker.item.estimate {"ref":"oxplow:oxplow/work_item?id=W-1&provider=fake","points":3}',
    'run tracker.item.flag {"ref":"oxplow:oxplow/work_item?id=W-1&provider=fake"}',
  ]);
});

test("a form about a ref opens with that ref as its param; a form about nothing is unchanged", () => {
  const { d, calls } = deps();
  const specs = [
    spec("triage.note.add", { label: "Add Note…", about: "triage/item", form: "oxplow:oxplow/page?page=triage/note-form" }),
    spec("triage.note.tag", { label: "Tag…", about: "triage/item", form: "oxplow:oxplow/page?mode=quick&page=triage/tag-form" }),
    spec("oxplow.thread.create", { label: "New Thread…", about: "triage/item", form: "new-thread" }),
  ];
  for (const o of refOffers(specs, "oxplow:triage/item?id=7", { streamId: null, threadId: null }, d)) o.run();
  expect(calls).toEqual([
    "open oxplow:oxplow/page?page=triage/note-form&ref=oxplow:triage/item?id%3D7",
    "open oxplow:oxplow/page?mode=quick&page=triage/tag-form&ref=oxplow:triage/item?id%3D7",
    "form new-thread",
  ]);
});
