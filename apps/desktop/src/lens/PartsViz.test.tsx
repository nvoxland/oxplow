import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

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
  path: "",
  tool: null,
} as Lens;

const lenses: Record<string, Lens> = {
  "x/open": { ...base, id: "x/open", slug: "open", title: "Open", viz: "number", params: [{ name: "stream_id", label: null, default: null, options: [] }] } as Lens,
  "x/none": { ...base, id: "x/none", slug: "none", title: "Nothing", viz: "table" } as Lens,
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
