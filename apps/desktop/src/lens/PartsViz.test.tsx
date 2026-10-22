import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Lens, LensPart, LensRun } from "../tauri-bridge/generated/bindings.js";

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

const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  getLens: async (id: string) => lenses[id]!,
  runLens: async (id: string, params: Record<string, unknown>, stream: string | null): Promise<LensRun> => {
    runs.push({ id, params, stream });
    const lens = lenses[id]!;
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
