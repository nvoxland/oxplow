import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { LensRun } from "../tauri-bridge/generated/bindings.js";

// An answer's page: what the agent showed, live, with Keep This until it
// is a lens, and then a link to that lens.

const realApi = { ...(await import("../api.js")) };
afterAll(() => {
  mock.module("../api.js", () => realApi);
});
const commands: [string, unknown][] = [];
const run = (lensId: string): LensRun =>
  ({
    lens: { id: lensId, extension: "", slug: "", title: "Churn", description: "", query: "", viz: "table", params: [], columns: [], parts: [], actions: [] },
    params: {},
    result: { columns: ["path"], rows: [["a.rs"]], truncated: false, reads: { models: [], tables: [], measures: [] } },
  }) as unknown as LensRun;
let shows = "answer/2";
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  runAnswer: async () => run(shows),
  runCommand: async (name: string, input: unknown) => {
    commands.push([name, input]);
    return { result: { lens: "my-lenses/churn" }, audit_id: 1, event_id: null, undo: null };
  },
}));
const { AnswerViz } = await import("./AnswerViz.js");

afterEach(() => {
  cleanup();
  commands.length = 0;
  shows = "answer/2";
});

const URI = "oxplow:oxplow/answer?id=2";

test("it runs the answer and draws what it shows", async () => {
  const view = render(<AnswerViz uri={URI} kept={null} />);
  await waitFor(() => expect(view.getByTestId("lens-answer-body").textContent).toContain("a.rs"));
});

test("Keep This asks for a name inline; Enter keeps it with lens.keep and links the lens; Escape cancels", async () => {
  const view = render(<AnswerViz uri={URI} kept={null} />);
  fireEvent.click(await waitFor(() => view.getByTestId("lens-answer-keep")));
  const slug = view.getByTestId("lens-answer-slug");
  fireEvent.keyDown(slug, { key: "Escape" });
  expect(view.queryByTestId("lens-answer-slug")).toBeNull();
  fireEvent.click(view.getByTestId("lens-answer-keep"));
  fireEvent.change(view.getByTestId("lens-answer-slug"), { target: { value: "churn" } });
  fireEvent.submit(view.getByTestId("lens-answer-slug").closest("form")!);
  await waitFor(() => expect(view.getByTestId("lens-answer-lens").textContent).toBe("Kept as my-lenses/churn"));
  expect(commands).toEqual([["oxplow.lens.keep", { answer: 2, slug: "churn" }]]);
});

test("a kept answer, or one that showed an existing lens, links that lens", async () => {
  const kept = render(<AnswerViz uri={URI} kept="my-lenses/churn" />);
  await waitFor(() => expect(kept.getByTestId("lens-answer-lens").textContent).toBe("Kept as my-lenses/churn"));
  cleanup();
  shows = "review/waiting";
  const existing = render(<AnswerViz uri={URI} kept={null} />);
  await waitFor(() => expect(existing.getByTestId("lens-answer-lens").textContent).toBe("Shows review/waiting"));
  expect(existing.queryByTestId("lens-answer-keep")).toBeNull();
});
