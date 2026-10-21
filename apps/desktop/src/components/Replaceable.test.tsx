import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { act, cleanup, render, waitFor } from "@testing-library/react";

// A core sub-component is replaced by an extension's lens: an unbound one
// (the Board) by the only extension replacing it or the person's choice; a
// bound one (a work item's state control) by its provider's extension
// only. Oxplow's own shows otherwise, and is what a failed replacement
// falls back to.

const realApi = await import("../api.js");
const listeners = new Set<(e: Record<string, unknown>) => void>();
const emit = (event: Record<string, unknown>) =>
  act(() => {
    for (const l of [...listeners]) l(event);
  });

let extensions: unknown[] = [];
/** The extension that brings the `fake` work-items provider (null: none). */
let fakeExtension: string | null = null;
/** The person's `replacements`, by target. */
let chosen: Record<string, string> = {};
let lensFails: string | null = null;
/** How long each next active-provider read takes to answer (ms). */
const providerDelays: number[] = [];
/** The replacement lens's viz: a `custom` one is a sandboxed bundle. */
let lensViz = "table";
const lensRuns: Array<[string, unknown]> = [];
const usage: Array<Record<string, unknown>> = [];

const providerRows = () => [
  ["work_items", "oxplow", null, "{}", 1],
  ["work_items", "fake", fakeExtension, "{}", 0],
  ["work_items", "other", "y", "{}", 0],
];

import { configValueResult, readsSettings } from "../configValuesTestSupport.js";

mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => extensions,
  subscribeOxplowEvents: (l: (e: Record<string, unknown>) => void) => {
    listeners.add(l);
    return () => listeners.delete(l);
  },
  querySql: async (sql: string) => {
    if (readsSettings(sql)) return configValueResult([{ key: "replacements", value: chosen }]);
    if (!sql.includes("v_capability_provider")) {
      return { columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} };
    }
    // What is active when it is asked, answered after its delay.
    const rows = providerRows();
    const delay = providerDelays.shift() ?? 0;
    if (delay > 0) await new Promise((r) => setTimeout(r, delay));
    return {
      columns: ["capability", "provider", "extension", "features", "active"],
      rows,
      truncated: false,
      reads: { models: ["v_capability_provider"], tables: [], measures: [] },
      freshness: {},
    };
  },
  runLens: async (id: string, params: unknown) => {
    lensRuns.push([id, params]);
    if (lensFails) throw new Error(lensFails);
    return {
      lens: {
        id,
        extension: id.split("/")[0],
        title: "Their Board",
        viz: lensViz,
        custom: lensViz === "custom" ? { component: "board", props: null } : null,
        columns: [],
        actions: [],
        parts: [],
        params: [],
      },
      params,
      result: { columns: ["title"], rows: [[`${id} row`]], truncated: false, reads: { models: ["v_work_item"], tables: [], measures: [] } },
      alert: null,
    };
  },
  recordUsage: async (u: Record<string, unknown>) => {
    usage.push(u);
  },
}));

const { Replaceable } = await import("./Replaceable.js");

/** Extension `name`, replacing the Board with its `board` lens. */
const replacing = (name: string) => ({
  name,
  enabled: true, pages: [],
  ui: {
    slots: [],
    commands: [],
    decorators: [],
    replacements: [
      { id: `${name}/work_item.board`, extension: name, target: "work_item.board", capability: "work_items", boundToProvider: false, lensId: `${name}/board`, label: "board", version: 1, view: [] },
    ],
  },
  lenses: [
    { id: `${name}/board`, params: ["scope", "thread_id"].map((p) => ({ name: p, label: null, default: null })) },
  ],
});

/** Extension `name`, replacing a work item's state control. */
const stating = (name: string) => ({
  name,
  enabled: true, pages: [],
  ui: {
    slots: [],
    commands: [],
    decorators: [],
    replacements: [
      { id: `${name}/work_item.detail.state`, extension: name, target: "work_item.detail.state", capability: "work_items", boundToProvider: true, lensId: `${name}/state`, label: "state control", version: 1, view: [] },
    ],
  },
  lenses: [{ id: `${name}/state`, params: [{ name: "ref", label: null, default: null }] }],
});

const PROPS = { scope: "all", thread_id: null };
const board = () => (
  <Replaceable target="work_item.board" props={PROPS} streamId="str1" fallback={<div data-testid="core-board">oxplow's board</div>} />
);
const STATE_PROPS = { ref: "work_item:fake:1" };
const state = (provider: string) => (
  <Replaceable
    target="work_item.detail.state"
    props={STATE_PROPS}
    streamId="str1"
    provider={provider}
    fallback={<div data-testid="core-state">oxplow's state</div>}
  />
);

beforeEach(() => {
  extensions = [];
  fakeExtension = null;
  chosen = {};
  lensFails = null;
  lensViz = "table";
  providerDelays.length = 0;
  lensRuns.length = 0;
  usage.length = 0;
});
afterEach(() => {
  cleanup();
  listeners.clear();
  // This file's `api.js` fakes outlive it (a module mock is process-wide):
  // leave them answering nothing, for whichever file runs next.
  extensions = [];
  fakeExtension = null;
  chosen = {};
  lensFails = null;
});

test("with nothing replacing it, the core component shows and no lens runs", async () => {
  const view = render(board());
  await waitFor(() => expect(view.getByTestId("core-board")).toBeTruthy());
  expect(view.container.querySelector('[data-testid^="replacement-"]')).toBeNull();
  expect(lensRuns).toEqual([]);
});

// The Board is a view of the capability as a whole: any extension may
// replace it. One shows by itself, given the target's props; among several
// the person chooses; "oxplow" keeps oxplow's own.
test("the Board shows its one replacement, or the person's choice among several", async () => {
  extensions = [replacing("x")];
  const one = render(board());
  const replaced = await waitFor(() => one.getByTestId("replacement-work_item.board"));
  expect(replaced.textContent).toContain("replaced by x");
  expect(replaced.textContent).toContain("x/board row");
  expect(lensRuns).toEqual([["x/board", PROPS]]);
  expect(usage).toEqual([{ kind: "replacement", key: "x/work_item.board", streamId: "str1" }]);
  cleanup();

  extensions = [replacing("x"), replacing("y")];
  lensRuns.length = 0;
  const several = render(board());
  await waitFor(() => expect(several.getByTestId("core-board")).toBeTruthy());
  expect(lensRuns).toEqual([]);

  chosen = { "work_item.board": "y" };
  emit({ kind: "configChanged" });
  await waitFor(() => expect(several.getByTestId("replacement-work_item.board").textContent).toContain("replaced by y"));

  chosen = { "work_item.board": "oxplow" };
  emit({ kind: "configChanged" });
  await waitFor(() => expect(several.getByTestId("core-board")).toBeTruthy());
  expect(several.container.querySelector('[data-testid^="replacement-"]')).toBeNull();
});

// tsk918: a work item's state control is its own provider's: only that
// provider's extension replaces it, however many extensions offer one.
test("a state control is replaced only by its item's provider's extension", async () => {
  extensions = [stating("x"), stating("y")];
  fakeExtension = "x";
  const view = render(state("fake"));
  const replaced = await waitFor(() => view.getByTestId("replacement-work_item.detail.state"));
  expect(replaced.textContent).toContain("replaced by x");
  expect(lensRuns).toEqual([["x/state", STATE_PROPS]]);
  cleanup();

  // oxplow's own item: no extension brings oxplow's provider.
  lensRuns.length = 0;
  const own = render(state("oxplow"));
  await waitFor(() => expect(own.getByTestId("core-state")).toBeTruthy());
  expect(lensRuns).toEqual([]);
});

test("the provider's extension changing swaps the component with no reload, and a person can keep oxplow's", async () => {
  extensions = [stating("x"), stating("y")];
  const view = render(state("fake"));
  await waitFor(() => expect(view.getByTestId("core-state")).toBeTruthy());

  fakeExtension = "y";
  emit({ kind: "modelsChanged", models: ["v_capability_provider"] });
  await waitFor(() => expect(view.getByTestId("replacement-work_item.detail.state").textContent).toContain("replaced by y"));
  expect(view.queryByTestId("core-state")).toBeNull();

  chosen = { "work_item.detail.state": "oxplow" };
  emit({ kind: "configChanged" });
  await waitFor(() => expect(view.getByTestId("core-state")).toBeTruthy());
  expect(view.container.querySelector('[data-testid^="replacement-"]')).toBeNull();
});

test("a replacement that can't load shows the core component and says why", async () => {
  extensions = [replacing("x")];
  lensFails = "no such view v_x_board";
  const view = render(board());
  const note = await waitFor(() => view.getByTestId("replacement-fallback"));
  expect(note.textContent).toContain("x");
  expect(note.textContent).toContain("no such view v_x_board");
  expect(note.textContent).toContain("Showing oxplow's");
  expect(view.getByTestId("core-board")).toBeTruthy();
  expect(view.queryByTestId("replacement-work_item.board")).toBeNull();
});

// tsk855: a custom replacement whose bundle can't load falls back the same
// way — outside its frame: no badge, none of the lens's toolbar, just the
// note and oxplow's own — and isn't counted as shown.
test("a custom replacement that can't load falls back outside its frame", async () => {
  extensions = [replacing("x")];
  lensViz = "custom";
  const view = render(board());
  const note = await waitFor(() => view.getByTestId("replacement-fallback"));
  expect(note.textContent).toContain("x's board couldn't load");
  expect(note.textContent).toContain("Showing oxplow's");
  expect(view.getByTestId("core-board")).toBeTruthy();
  expect(view.queryByTestId("replacement-work_item.board")).toBeNull();
  expect(view.queryByTestId("replacement-badge")).toBeNull();
  expect(usage).toEqual([]);
});

// tsk857: reads of whose provider it is can overlap (a change each); the
// newest one asked decides, however they resolve.
test("an older provider read answering late doesn't win", async () => {
  extensions = [stating("x"), stating("y")];
  fakeExtension = "x";
  const view = render(state("fake"));
  await waitFor(() => expect(view.getByTestId("replacement-work_item.detail.state").textContent).toContain("replaced by x"));

  // A change starts a read once a burst is over (`COALESCE_MS`, 100).
  providerDelays.push(300, 0);
  fakeExtension = "y";
  emit({ kind: "modelsChanged", models: ["v_capability_provider"] });
  // That read is under way (slow) when the next change is heard.
  await act(() => new Promise((r) => setTimeout(r, 150)));
  fakeExtension = null;
  emit({ kind: "modelsChanged", models: ["v_capability_provider"] });
  await waitFor(() => expect(view.getByTestId("core-state")).toBeTruthy());
  // The older read answers last.
  await act(() => new Promise((r) => setTimeout(r, 400)));
  expect(view.getByTestId("core-state")).toBeTruthy();
  expect(view.container.querySelector('[data-testid^="replacement-"]')).toBeNull();
});


/** Extension `name`, drawing the history graph (a target no provider binds). */
const graphing = (name: string) => ({
  name,
  enabled: true, pages: [],
  ui: {
    slots: [],
    commands: [],
    decorators: [],
    replacements: [
      { id: `${name}/vcs.history.graph`, extension: name, target: "vcs.history.graph", capability: "vcs", boundToProvider: false, lensId: `${name}/graph`, label: "history graph", version: 2, view: ["q", "author", "branch"] },
    ],
  },
  lenses: [
    { id: `${name}/graph`, params: ["head_sha", "reveal_sha", "limit"].map((p) => ({ name: p, label: null, default: null })) },
  ],
});
const GRAPH_PROPS = { head_sha: "abc", reveal_sha: null, limit: 500 };
const graph = () => (
  <Replaceable target="vcs.history.graph" props={GRAPH_PROPS} streamId="str1" fallback={<div data-testid="core-list">oxplow's list</div>} />
);

// A target core owns isn't bound to a provider: the one extension
// replacing it shows (no provider read), several show only the one the
// person chose, and none chosen keeps oxplow's own.
test("an unbound target shows its one replacement, or the person's choice among several", async () => {
  extensions = [graphing("oxplow-bundled")];
  const one = render(graph());
  await waitFor(() => expect(one.getByTestId("replacement-vcs.history.graph").textContent).toContain("replaced by oxplow-bundled"));
  expect(lensRuns).toEqual([["oxplow-bundled/graph", GRAPH_PROPS]]);
  cleanup();

  extensions = [graphing("oxplow-bundled"), graphing("lanes")];
  lensRuns.length = 0;
  const several = render(graph());
  await waitFor(() => expect(several.getByTestId("core-list")).toBeTruthy());
  expect(lensRuns).toEqual([]);

  chosen = { "vcs.history.graph": "lanes" };
  emit({ kind: "configChanged" });
  await waitFor(() => expect(several.getByTestId("replacement-vcs.history.graph").textContent).toContain("replaced by lanes"));

  chosen = { "vcs.history.graph": "oxplow" };
  emit({ kind: "configChanged" });
  await waitFor(() => expect(several.getByTestId("core-list")).toBeTruthy());
});

// An effort's verdict is a review policy's: oxplow has none of its own, so
// the page shows nothing until an extension brings one — then its lens,
// given the effort's `effort_id`; "oxplow" chosen shows none again.
test("an effort's verdict is only ever an extension's", async () => {
  const judging = (name: string) => ({
    name,
    enabled: true, pages: [],
    ui: {
      slots: [],
      commands: [],
      decorators: [],
      replacements: [
        { id: `${name}/effort.verdict`, extension: name, target: "effort.verdict", capability: "effort_policy", boundToProvider: false, lensId: `${name}/effort-verdict`, label: "effort verdict", version: 1, view: [] },
      ],
    },
    lenses: [{ id: `${name}/effort-verdict`, params: [{ name: "effort_id", label: null, default: null }] }],
  });
  const verdict = () => <Replaceable target="effort.verdict" props={{ effort_id: 7 }} streamId="str1" fallback={null} />;
  const none = render(verdict());
  await waitFor(() => expect(none.container.innerHTML).toBe(""));
  expect(lensRuns).toEqual([]);
  cleanup();

  extensions = [judging("oxplow-bundled")];
  const one = render(verdict());
  const shown = await waitFor(() => one.getByTestId("replacement-effort.verdict"));
  expect(shown.textContent).toContain("oxplow-bundled/effort-verdict row");
  expect(lensRuns[0]).toEqual(["oxplow-bundled/effort-verdict", { effort_id: 7 }]);
  cleanup();

  chosen = { "effort.verdict": "oxplow" };
  const off = render(verdict());
  await waitFor(() => expect(off.container.querySelector('[data-testid^="replacement-"]')).toBeNull());
  expect(off.container.innerHTML).toBe("");
});
