import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { subscribeAgentInput } from "../../agent-input-bus.js";
import { askText, promptsAbout, type CatalogPrompt } from "./promptModel.js";
import { promptsOf } from "./usePromptCatalog.js";

// P6.D2: the prompt catalog, contextual suggestions, and EmptyState —
// every Ask fills the agent's input and never sends.

const catalog: CatalogPrompt[] = [
  { prompt: "Which files did this effort touch?", about: "effort", source: { kind: "area", name: "work_items" } },
  { prompt: "Who has changed this file the most?", about: "file", source: { kind: "area", name: "vcs" } },
  { prompt: "Which PRs wait on me?", about: null, source: { kind: "extension", name: "gh" } },
];

const realApi = await import("../../api.js");
// What `v_prompt` answers, as rows.
const rows = catalog.map((p) => [p.source.kind, p.source.name, p.prompt, p.about]);
mock.module("../../api.js", () => ({
  ...realApi,
  querySql: async (sql: string) =>
    sql.includes("v_prompt")
      ? { columns: ["source_kind", "source", "prompt", "about"], rows, truncated: false, reads: { models: ["v_prompt"], tables: [], measures: [] } }
      : realApi.querySql(sql),
}));
const { SuggestedPrompts } = await import("./SuggestedPrompts.js");
const { EmptyState } = await import("./EmptyState.js");

const inserted: string[] = [];
let off: () => void = () => {};
beforeEach(() => {
  inserted.length = 0;
  off = subscribeAgentInput((t) => inserted.push(t));
});
afterEach(() => {
  off();
  cleanup();
});

test("v_prompt's rows are the catalog; prompts about a kind; asking about a ref leads with it", () => {
  expect(promptsOf(rows)).toEqual(catalog);
  expect(promptsAbout(catalog, "effort").map((p) => p.prompt)).toEqual(["Which files did this effort touch?"]);
  expect(askText("Who has changed this file the most?", "oxplow:oxplow/file?path=src/a.rs")).toBe(
    "[oxplow ref oxplow:oxplow/file?path=src/a.rs] Who has changed this file the most?",
  );
  expect(askText("Which PRs wait on me?", null)).toBe("Which PRs wait on me?");
});

test("a page for an effort suggests its prompts, asked about that effort", async () => {
  const view = render(<SuggestedPrompts refId="oxplow:oxplow/effort?id=12" />);
  const ask = await waitFor(() => view.getByText("Which files did this effort touch?"));
  expect(view.queryByText("Who has changed this file the most?")).toBeNull();
  fireEvent.click(ask);
  expect(inserted).toEqual(["[oxplow ref oxplow:oxplow/effort?id=12] Which files did this effort touch?"]);
});

test("an empty state offers prompts; Ask fills the input", () => {
  const view = render(
    <EmptyState testId="x-empty" title="No dashboards yet" text="A dashboard is a grid of tiles." prompts={["Build a dashboard of test health"]} />,
  );
  expect(view.getByTestId("x-empty").textContent).toContain("No dashboards yet");
  fireEvent.click(view.getByText("Build a dashboard of test health"));
  expect(inserted).toEqual(["Build a dashboard of test health"]);
});
