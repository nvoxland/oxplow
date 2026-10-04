import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { act, cleanup, render, waitFor } from "@testing-library/react";

// P9.A1: a core sub-component (the Board) is replaced by the lens of the
// capability's ACTIVE provider's extension — and only that one; oxplow's
// own is what shows otherwise, and what a failed replacement falls back to.

const realApi = await import("../api.js");
const listeners = new Set<(e: Record<string, unknown>) => void>();
const emit = (event: Record<string, unknown>) =>
  act(() => {
    for (const l of [...listeners]) l(event);
  });

let extensions: unknown[] = [];
/** The active work-items provider's extension (null: oxplow's own). */
let activeExtension: string | null = null;
let off: string[] = [];
let lensFails: string | null = null;
/** How long each next active-provider read takes to answer (ms). */
const providerDelays: number[] = [];
/** The replacement lens's viz: a `custom` one is a sandboxed bundle. */
let lensViz = "table";
const lensRuns: Array<[string, unknown]> = [];
const usage: Array<Record<string, unknown>> = [];

const providerRows = () => [
  ["work_items", "oxplow", null, "{}", activeExtension === null ? 1 : 0],
  ["work_items", "fake", "x", "{}", activeExtension === "x" ? 1 : 0],
  ["work_items", "other", "y", "{}", activeExtension === "y" ? 1 : 0],
];

mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => extensions,
  subscribeOxplowEvents: (l: (e: Record<string, unknown>) => void) => {
    listeners.add(l);
    return () => listeners.delete(l);
  },
  effectiveConfig: async () => [{ key: "replacementsOff", value: off }],
  querySql: async (sql: string) => {
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
        children: [],
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
  enabled: true,
  ui: {
    slots: [],
    commands: [],
    decorators: [],
    replacements: [
      { id: `${name}/work_item.board`, extension: name, target: "work_item.board", capability: "work_items", lensId: `${name}/board`, label: "board" },
    ],
  },
  lenses: [
    { id: `${name}/board`, params: ["scope", "thread_id"].map((p) => ({ name: p, label: null, default: null })) },
  ],
});

const PROPS = { scope: "all", thread_id: null };
const board = () => (
  <Replaceable target="work_item.board" props={PROPS} streamId="str1" fallback={<div data-testid="core-board">oxplow's board</div>} />
);

beforeEach(() => {
  extensions = [];
  activeExtension = null;
  off = [];
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
  activeExtension = null;
  off = [];
  lensFails = null;
});

test("with nothing replacing it, the core component shows and no lens runs", async () => {
  const view = render(board());
  await waitFor(() => expect(view.getByTestId("core-board")).toBeTruthy());
  expect(view.container.querySelector('[data-testid^="replacement-"]')).toBeNull();
  expect(lensRuns).toEqual([]);
});

test("only the active provider's replacement renders, given the target's props", async () => {
  extensions = [replacing("x"), replacing("y")];
  activeExtension = "x";
  const view = render(board());
  const replaced = await waitFor(() => view.getByTestId("replacement-work_item.board"));
  expect(replaced.textContent).toContain("replaced by x");
  expect(replaced.textContent).toContain("x/board row");
  expect(view.queryByTestId("core-board")).toBeNull();
  expect(lensRuns).toEqual([["x/board", PROPS]]);
  expect(usage).toEqual([{ kind: "replacement", key: "x/work_item.board", streamId: "str1" }]);
});

test("another provider's extension replaces nothing while it isn't the active one", async () => {
  extensions = [replacing("x"), replacing("y")];
  activeExtension = null;
  const view = render(board());
  await waitFor(() => expect(view.getByTestId("core-board")).toBeTruthy());
  expect(lensRuns).toEqual([]);
});

test("the active provider changing swaps the component with no reload, and a person can turn it off", async () => {
  extensions = [replacing("x"), replacing("y")];
  const view = render(board());
  await waitFor(() => expect(view.getByTestId("core-board")).toBeTruthy());

  activeExtension = "y";
  emit({ kind: "modelsChanged", models: ["v_capability_provider"] });
  await waitFor(() => expect(view.getByTestId("replacement-work_item.board").textContent).toContain("replaced by y"));
  expect(view.queryByTestId("core-board")).toBeNull();

  off = ["work_item.board"];
  emit({ kind: "configChanged" });
  await waitFor(() => expect(view.getByTestId("core-board")).toBeTruthy());
  expect(view.container.querySelector('[data-testid^="replacement-"]')).toBeNull();
});

test("a replacement that can't load shows the core component and says why", async () => {
  extensions = [replacing("x")];
  activeExtension = "x";
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
  activeExtension = "x";
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

// tsk857: reads of who is active can overlap (a change each); the newest
// one asked decides, however they resolve.
test("an older active-provider read answering late doesn't win", async () => {
  extensions = [replacing("x"), replacing("y")];
  activeExtension = "x";
  const view = render(board());
  await waitFor(() => expect(view.getByTestId("replacement-work_item.board").textContent).toContain("replaced by x"));

  // A change starts a read once a burst is over (`COALESCE_MS`, 100).
  providerDelays.push(300, 0);
  activeExtension = "y";
  emit({ kind: "modelsChanged", models: ["v_capability_provider"] });
  // That read is under way (slow) when the next change is heard.
  await act(() => new Promise((r) => setTimeout(r, 150)));
  activeExtension = null;
  emit({ kind: "modelsChanged", models: ["v_capability_provider"] });
  await waitFor(() => expect(view.getByTestId("core-board")).toBeTruthy());
  // The older read answers last.
  await act(() => new Promise((r) => setTimeout(r, 400)));
  expect(view.getByTestId("core-board")).toBeTruthy();
  expect(view.container.querySelector('[data-testid^="replacement-"]')).toBeNull();
});

