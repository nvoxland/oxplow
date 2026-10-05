import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { subscribeAgentInput } from "../../agent-input-bus.js";
import type { CatalogPrompt } from "../../tauri-bridge/generated/bindings.js";
import { askText, promptsAbout, promptsBySource } from "./promptModel.js";

// P6.D2: the prompt catalog, contextual suggestions, and EmptyState —
// every Ask fills the agent's input and never sends.

const catalog: CatalogPrompt[] = [
  { prompt: "Which files did this effort touch?", about: "effort", source: { kind: "capability", name: "work_items" } },
  { prompt: "Who has changed this file the most?", about: "file", source: { kind: "capability", name: "vcs" } },
  { prompt: "Which PRs wait on me?", about: null, source: { kind: "extension", name: "gh" } },
];

const realApi = await import("../../api.js");
mock.module("../../api.js", () => ({ ...realApi, promptCatalog: async () => catalog }));
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

test("prompts about a kind; grouped by source, core first; asking about a ref leads with it", () => {
  expect(promptsAbout(catalog, "effort").map((p) => p.prompt)).toEqual(["Which files did this effort touch?"]);
  // A capability reads as a person names it (tsk1044); its id stays the key.
  expect(promptsBySource(catalog).map((g) => [g.name, g.label, g.prompts.length])).toEqual([
    ["vcs", "Version control", 1],
    ["work_items", "Work items", 1],
    ["gh", "gh", 1],
  ]);
  expect(askText("Who has changed this file the most?", "file:src/a.rs")).toBe(
    "[oxplow ref file:src/a.rs] Who has changed this file the most?",
  );
  expect(askText("Which PRs wait on me?", null)).toBe("Which PRs wait on me?");
});

test("a page for an effort suggests its prompts, asked about that effort", async () => {
  const view = render(<SuggestedPrompts refId="effort:12" streamId={null} />);
  const ask = await waitFor(() => view.getByText("Which files did this effort touch?"));
  expect(view.queryByText("Who has changed this file the most?")).toBeNull();
  fireEvent.click(ask);
  expect(inserted).toEqual(["[oxplow ref effort:12] Which files did this effort touch?"]);
});

test("an empty state offers prompts; Ask fills the input", () => {
  const view = render(
    <EmptyState testId="x-empty" title="No dashboards yet" text="A dashboard is a grid of tiles." prompts={["Build a dashboard of test health"]} />,
  );
  expect(view.getByTestId("x-empty").textContent).toContain("No dashboards yet");
  fireEvent.click(view.getByText("Build a dashboard of test health"));
  expect(inserted).toEqual(["Build a dashboard of test health"]);
});
