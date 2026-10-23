import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, createEvent, fireEvent, render, waitFor } from "@testing-library/react";

import type { Lens, LensAction, LensPart, LensRun } from "../tauri-bridge/generated/bindings.js";
import { LENS_ROW_DRAG_MIME } from "../dragMimes.js";

// A document's sections and a dashboard's tiles are other lenses (or
// prose), each run with the params it declares from the page's, in its
// stream.

const base: Lens = {
  id: "x/l",
  extension: "x",
  slug: "l",
  title: "L",
  description: "",
  query: "",
  viz: "table",
  params: [],
  columns: [],
  empty: null,
  chart: null,
  tree: null,
  tile: null,
  timeline: null,
  steps: null,
  hunks: null,
  form: null,
  custom: null,
  group: null,
  emphasis: null,
  depth: null,
  parts: [],
  launcherCategory: null,
  hidden: false,
  actions: [],
  alert: null,
  needs: [],
  origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
  tool: null,
} as Lens;

const lenses: Record<string, Lens> = {
  "x/open": { ...base, id: "x/open", slug: "open", title: "Open", viz: "number", params: [{ name: "stream_id", label: null, default: null, options: [] }] } as Lens,
  "x/none": { ...base, id: "x/none", slug: "none", title: "Nothing", viz: "table" } as Lens,
  "x/off": { ...base, id: "x/off", slug: "off", title: "Off", viz: "number" } as Lens,
  "x/cards": { ...base, id: "x/cards", slug: "cards", title: "Cards", viz: "number", params: [{ name: "change_id", label: null, default: null, options: [] }] } as Lens,
  "x/pick": {
    ...base,
    id: "x/pick",
    slug: "pick",
    title: "Pick",
    params: [{ name: "commit", label: null, default: null, options: [], type: "text", role: "selection", presets: [] }],
    columns: [{ key: "n", label: null, link: null, unit: null, icon: null, tone: null, select: "commit" }],
  } as unknown as Lens,
  "x/detail": {
    ...base,
    id: "x/detail",
    slug: "detail",
    title: "Detail",
    params: [{ name: "commit", label: null, default: null, options: [], type: "text", role: "selection", presets: [] }],
  } as Lens,
};
const runs: { id: string; params: unknown; stream: string | null }[] = [];
const ran: { action: string; row: unknown; input: unknown; drop: unknown }[] = [];

const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  getLens: async (id: string) => lenses[id]!,
  querySql: async () => ({ columns: ["answer"], rows: [[42]], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} }),
  runLensAction: async (_id: string, action: string, _params: unknown, row: unknown, _s: unknown, _confirmed: boolean, input: unknown, drop: unknown) => {
    ran.push({ action, row, input, drop });
    return { result: null, audit_id: 1, event_id: null, inverse: null };
  },
  runLens: async (id: string, params: Record<string, unknown>, stream: string | null): Promise<LensRun> => {
    runs.push({ id, params, stream });
    const lens = lenses[id]!;
    if (id === "x/off") {
      return {
        lens,
        params: params as LensRun["params"],
        result: { columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} },
        inactive: { message: "Needs a work list with links.", needs: ["work_items.links"] },
      } as unknown as LensRun;
    }
    return {
      lens,
      params: params as LensRun["params"],
      result: { columns: ["n"], rows: id === "x/none" ? [] : [[7]], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} },
    } as unknown as LensRun;
  },
}));

const { LensResultView } = await import("./LensResultView.js");

afterEach(() => {
  cleanup();
  runs.length = 0;
  ran.length = 0;
});

const part = (over: Partial<LensPart>): LensPart => ({ id: "p", title: null, lens: null, markdown: null, size: null, from: null, ...over });
const page = (viz: "document" | "dashboard", parts: LensPart[]): LensRun =>
  ({ lens: { ...base, viz, parts }, params: { stream_id: 3, other: "x" }, result: { columns: [], rows: [], truncated: false } }) as unknown as LensRun;

test("a document shows its parts as sections, runs each with its own params, and says whose an added one is", async () => {
  const { findByText, getByTestId } = render(
    <LensResultView
      run={page("document", [
        part({ id: "intro", title: "About", markdown: "Read this first." }),
        part({ id: "open", lens: "x/open", from: "addon" }),
        part({ id: "none", lens: "x/none" }),
      ])}
      streamId="str1"
      onOpenPage={() => {}}
    />,
  );
  expect(await findByText("Read this first.")).toBeTruthy();
  await waitFor(() => expect(getByTestId("lens-part-open").textContent).toContain("7"));
  const open = getByTestId("lens-part-open");
  expect(open.querySelector("h2")!.textContent).toContain("Open");
  expect(open.querySelector("[data-source]")!.textContent).toBe("addon");
  expect(runs.find((r) => r.id === "x/open")).toEqual({ id: "x/open", params: { stream_id: 3 }, stream: "str1" });
  // A section with nothing to show folds into the closing line.
  expect(getByTestId("lens-document-nothing-found").textContent).toContain("Nothing");
});

test("a dashboard shows its parts as tiles at their sizes", async () => {
  const { getByTestId } = render(
    <LensResultView run={page("dashboard", [part({ id: "open", lens: "x/open", size: "wide" }), part({ id: "note", markdown: "hi" })])} streamId="str1" onOpenPage={() => {}} />,
  );
  await waitFor(() => expect(getByTestId("lens-part-open").textContent).toContain("7"));
  expect(getByTestId("lens-part-open").className).toContain("ox-tile-wide");
  expect(getByTestId("lens-part-note").className).toContain("ox-tile-full");
});

/** A section's row click sets the document's param, and the sections that
 *  read it run again with the new value: master and detail. */
test("a click in one section sets the document's param, and the others re-run with it", async () => {
  const picked: [string, unknown][] = [];
  const doc = (commit: unknown) =>
    ({
      lens: { ...base, viz: "document", parts: [part({ id: "pick", lens: "x/pick" }), part({ id: "detail", lens: "x/detail" })] },
      params: { commit },
      result: { columns: [], rows: [], truncated: false },
    }) as unknown as LensRun;
  const view = render(<LensResultView run={doc(null)} streamId="str1" onOpenPage={() => {}} onSetParam={(n, v) => picked.push([n, v])} />);
  await waitFor(() => expect(view.getByTestId("lens-part-pick").textContent).toContain("7"));
  fireEvent.click(view.getByTestId("lens-part-pick").querySelector("[data-testid=lens-row-0]")!);
  expect(picked).toEqual([["commit", 7]]);
  view.rerender(<LensResultView run={doc(7)} streamId="str1" onOpenPage={() => {}} onSetParam={() => {}} />);
  await waitFor(() => expect(runs.some((r) => r.id === "x/detail" && (r.params as { commit?: unknown }).commit === 7)).toBe(true));
});

/** A document's query is one row: its columns are what its sections are
 *  run with, each taking the ones it declares. */
test("a document's row is its sections' params", async () => {
  const doc = {
    lens: { ...base, viz: "document", parts: [part({ id: "cards", lens: "x/cards" })] },
    params: { stream_id: 3 },
    result: { columns: ["change_id", "stray"], rows: [[42, "x"]], truncated: false },
  } as unknown as LensRun;
  const view = render(<LensResultView run={doc} streamId="str1" onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getByTestId("lens-part-cards").textContent).toContain("7"));
  expect(runs.find((r) => r.id === "x/cards")).toEqual({ id: "x/cards", params: { change_id: 42 }, stream: "str1" });
});

test("a section that needs what isn't active is left out, and isn't counted as nothing found", async () => {
  const { getByTestId, queryByTestId, queryByText } = render(
    <LensResultView run={page("document", [part({ id: "open", lens: "x/open" }), part({ id: "off", lens: "x/off" })])} streamId="str1" onOpenPage={() => {}} />,
  );
  await waitFor(() => expect(getByTestId("lens-part-open").textContent).toContain("7"));
  expect(queryByTestId("lens-part-off")).toBeNull();
  expect(queryByTestId("lens-document-nothing-found")).toBeNull();
  expect(queryByText(/Needs a work list/)).toBeNull();
});

// A dashboard whose rows are its tiles (`tile:`): one tile per row after
// its static ones, which drag to reorder, right-click and take keys.

const action = (over: Partial<LensAction>): LensAction =>
  ({ id: "x", label: "X", command: "oxplow.dashboard.update_item", input: {}, row: true, group: null, form: false, fixed: [], choices: {}, then: "open", when: null, primary: false, drag: false, bulk: false, key: null, edit: false, editField: null, ...over }) as LensAction;
const reorder = action({ id: "reorder", label: "Reorder", command: "oxplow.dashboard.reorder_items", drag: true, input: { dashboard: "{{param.id}}", order: "{{order.id}}" } });
const remove = action({ id: "remove", label: "Remove", command: "oxplow.dashboard.remove_item", key: "Delete", input: { item: "{{row.id}}" } });
const editText = action({ id: "edit-text", label: "Save", edit: true, fixed: ["item"], editField: "text", input: { item: "{{row.id}}", text: "{{row.text}}" } });

const tileColumns = ["id", "title", "size", "lens", "sql", "display", "text"];
const tileRows = [
  ["dti1", "Open work", "wide", "x/open", null, null, null],
  ["dti2", null, null, null, "SELECT 42 AS answer", "number", null],
  ["dti3", null, null, null, null, null, "Section"],
];
const tiled = (actions: LensAction[]): LensRun =>
  ({
    lens: {
      ...base,
      viz: "dashboard",
      parts: [part({ id: "note", markdown: "static" })],
      tile: { id: "id", title: "title", size: "size", lens: "lens", sql: "sql", display: "display", chart: null, metric: null, viz: null, text: "text" },
      actions,
    },
    params: { id: "dsh1", stream_id: 3 },
    result: { columns: tileColumns, rows: tileRows, truncated: false, reads: { models: [], tables: [], measures: [] } },
  }) as unknown as LensRun;
const record = (i: number) => Object.fromEntries(tileColumns.map((c, j) => [c, tileRows[i]![j] ?? null]));

/** `el` as a 100px-wide tile at the left, so a drop's x picks its side. */
function wide(el: HTMLElement): HTMLElement {
  el.getBoundingClientRect = () => ({ top: 0, height: 100, bottom: 100, left: 0, right: 100, width: 100, x: 0, y: 0, toJSON: () => ({}) });
  return el;
}

function drag(from: HTMLElement) {
  const data: Record<string, string> = {};
  const dataTransfer = {
    setData: (k: string, v: string) => (data[k] = v),
    getData: (k: string) => data[k] ?? "",
    get types() {
      return Object.keys(data);
    },
    effectAllowed: "",
    dropEffect: "",
  };
  fireEvent.dragStart(from, { dataTransfer });
  return {
    data,
    over(to: HTMLElement, x: number) {
      const ev = createEvent.dragOver(to, { dataTransfer });
      Object.defineProperty(ev, "clientX", { value: x });
      fireEvent(to, ev);
    },
    dropOn(to: HTMLElement, x = 0) {
      for (const make of [createEvent.dragOver, createEvent.drop]) {
        const ev = make(to, { dataTransfer });
        Object.defineProperty(ev, "clientX", { value: x });
        fireEvent(to, ev);
      }
    },
  };
}

test("a dashboard's rows are tiles after its static ones, each at its size, drawn by its lens, sql or text", async () => {
  const view = render(<LensResultView run={tiled([])} streamId="str1" onOpenPage={() => {}} />);
  const grid = view.getByTestId("lens-dashboard");
  const ids = Array.from(grid.children).map((c) => c.getAttribute("data-testid"));
  expect(ids).toEqual(["lens-part-note", "lens-tile-dti1", "lens-tile-dti2", "lens-tile-dti3"]);
  await waitFor(() => expect(view.getByTestId("lens-tile-dti1").textContent).toContain("7"));
  expect(view.getByTestId("lens-tile-dti1").textContent).toContain("Open work");
  expect(view.getByTestId("lens-tile-dti1").className).toContain("ox-tile-wide");
  // A lens tile runs with the page's params it declares.
  expect(runs.find((r) => r.id === "x/open")).toEqual({ id: "x/open", params: { stream_id: 3 }, stream: "str1" });
  await waitFor(() => expect(view.getByTestId("lens-tile-dti2").textContent).toContain("42"));
  expect(view.getByTestId("lens-tile-dti3").textContent).toBe("Section");
  expect(view.getByTestId("lens-tile-dti3").className).toContain("ox-tile-full");
  // A heading without an edit action is read-only.
  expect(view.getByTestId("lens-tile-dti3").querySelector("input")).toBeNull();
});

test("right-click on a row tile offers its row actions, and a key runs one", async () => {
  const view = render(<LensResultView run={tiled([remove])} streamId="str1" onOpenPage={() => {}} />);
  const tile = view.getByTestId("lens-tile-dti2");
  fireEvent.contextMenu(tile);
  expect(view.getByText("Remove")).toBeTruthy();
  expect(view.getByText("Ask About This")).toBeTruthy();
  fireEvent.keyDown(document.body, { key: "Escape" });
  fireEvent.keyDown(tile, { key: "Delete" });
  await waitFor(() => expect(ran).toHaveLength(1));
  expect(ran[0]).toMatchObject({ action: "remove", row: record(1) });
});

test("a tile dragged beside another runs the drag action with every tile in its new order", async () => {
  const view = render(<LensResultView run={tiled([reorder])} streamId="str1" onOpenPage={() => {}} />);
  const first = view.getByTestId("lens-tile-dti1");
  expect(first.getAttribute("draggable")).toBe("true");
  const d = drag(first);
  expect(JSON.parse(d.data[LENS_ROW_DRAG_MIME]!).keys).toEqual(["dti1"]);
  const last = wide(view.getByTestId("lens-tile-dti3"));
  d.over(last, 75);
  expect(last.getAttribute("data-drop")).toBe("after");
  d.dropOn(last, 75);
  await waitFor(() => expect(ran).toHaveLength(1));
  expect(ran[0]).toMatchObject({ action: "reorder", row: record(0), drop: { after: record(2), order: [record(1), record(2), record(0)] } });
  expect(last.getAttribute("data-drop")).toBeNull();
  // A drop on the grid's background puts the tile last.
  ran.length = 0;
  drag(view.getByTestId("lens-tile-dti2")).dropOn(view.getByTestId("lens-dashboard"));
  await waitFor(() => expect(ran).toHaveLength(1));
  expect(ran[0]).toMatchObject({ action: "reorder", drop: { order: [record(0), record(2), record(1)] } });
});

test("a heading tile with an edit action is edited in place and saved through it", async () => {
  const view = render(<LensResultView run={tiled([editText])} streamId="str1" onOpenPage={() => {}} />);
  const band = view.getByTestId("lens-tile-dti3-text");
  fireEvent.click(band);
  const input = view.getByTestId("lens-tile-dti3-text") as HTMLInputElement;
  fireEvent.change(input, { target: { value: "Renamed" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(ran).toHaveLength(1));
  expect(ran[0]).toMatchObject({ action: "edit-text", row: record(2), input: { text: "Renamed" } });
});
