import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

// A core document page's sections are a slot: what an extension mounts
// there lands where its `at` says, says whose it is, and runs with the
// params the page binds.

const lens = (id: string, title: string) => ({ id, title, params: [{ name: "thread_id" }] });
const runs: { id: string; params: unknown }[] = [];
const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [
    {
      name: "addon",
      enabled: true, pages: [],
      lenses: [lens("addon/presets", "Presets"), lens("addon/tips", "Tips")],
      ui: {
        slots: [
          { slot: "demo.sections", lensId: "addon/presets", at: { after: "start" }, title: "Team presets", size: null },
          { slot: "demo.sections", lensId: "addon/tips", at: null, title: null, size: null },
          { slot: "elsewhere.sections", lensId: "addon/tips", at: "start", title: null, size: null },
        ],
        decorators: [],
        replacements: [],
      },
    },
  ],
  getLens: async (id: string) => lens(id, id),
  runLens: async (id: string, params: unknown) => {
    runs.push({ id, params });
    return { lens: { ...lens(id, id), viz: "number", columns: [], actions: [], parts: [] }, params, result: { columns: ["n"], rows: [[1]], truncated: false, reads: { models: [], tables: [], measures: [] } } };
  },
}));

const { DocumentLayout } = await import("./DocumentLayout.js");

afterEach(cleanup);

test("mounted sections land among the page's own by their `at`, marked with their extension", async () => {
  const { container, getByTestId } = render(
    <DocumentLayout
      title="Demo"
      slot="demo.sections"
      slotParams={{ thread_id: 7, other: 1 }}
      sections={[
        { id: "start", title: "Start", body: <p>start body</p> },
        { id: "earlier", title: "Earlier", body: <p>earlier body</p> },
      ]}
    />,
  );
  await waitFor(() => expect(container.querySelectorAll("section").length).toBe(4));
  expect([...container.querySelectorAll("section")].map((s) => s.getAttribute("data-section"))).toEqual(["start", "addon/presets", "earlier", "addon/tips"]);
  expect(getByTestId("demo.sections-addon/presets").querySelector("h2")!.textContent).toContain("Team presets");
  expect(getByTestId("demo.sections-addon/presets").querySelector("[data-source]")!.textContent).toBe("addon");
  await waitFor(() => expect(runs.find((r) => r.id === "addon/presets")?.params).toEqual({ thread_id: 7 }));
  // Four sections: a contents list in the rail.
  expect(getByTestId("document-contents").textContent).toContain("Team presets");
});
