import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { promptsAbout, type CatalogPrompt } from "./promptModel.js";
import { promptsOf } from "./usePromptCatalog.js";

// P6.D2: the prompt catalog, contextual suggestions, and EmptyState —
// every Ask fills the Assistant's box and never sends.

const catalog: CatalogPrompt[] = [
  { prompt: "Which files did this effort touch?", about: "oxplow/effort", source: { kind: "extension", name: "oxplow-foundation" } },
  { prompt: "Who has changed this file the most?", about: "oxplow/file", source: { kind: "extension", name: "oxplow-foundation" } },
  { prompt: "Which PRs wait on me?", about: null, source: { kind: "extension", name: "gh" } },
];

const realApi = { ...(await import("../../api.js")) }; // a copy: `mock.module` patches a loaded module in place
// What `v_page_answer` answers, as rows.
const rows = catalog.map((p) => [p.source.kind, p.source.name, p.prompt, p.about]);
mock.module("../../api.js", () => ({
  ...realApi,
  querySql: async (sql: string) =>
    sql.includes("v_page_answer")
      ? { columns: ["source_kind", "source", "prompt", "about"], rows, truncated: false, reads: { datasets: ["v_page_answer"], tables: [], measures: [] } }
      : realApi.querySql(sql),
}));
const { EmptyState } = await import("./EmptyState.js");

const { getAssistant, resetAssistant } = await import("../Assistant/assistantSession.js");
const { dockStore } = await import("../Dock/dockStore.js");
afterEach(() => {
  cleanup();
  resetAssistant();
  dockStore.close();
});

test("v_page_answer's rows are the catalog; prompts about a kind", () => {
  expect(promptsOf(rows)).toEqual(catalog);
  expect(promptsAbout(catalog, "oxplow/effort").map((p) => p.prompt)).toEqual(["Which files did this effort touch?"]);
});

test("an empty state offers prompts; Ask puts it in the Assistant's box", () => {
  const view = render(
    <EmptyState testId="x-empty" title="No dashboards yet" text="A dashboard is a grid of tiles." prompts={["Build a dashboard of test health"]} />,
  );
  expect(view.getByTestId("x-empty").textContent).toContain("No dashboards yet");
  fireEvent.click(view.getByText("Build a dashboard of test health"));
  expect(getAssistant().draft?.text).toBe("Build a dashboard of test health");
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
