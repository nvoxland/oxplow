import { afterAll, expect, mock, test } from "bun:test";
import { act, render, waitFor } from "@testing-library/react";

// A burst of commits touching knowledge pages (a scan storing its findings
// writes page refs) is one re-read of the page list, not one per commit —
// and a change while a read is running is still read afterwards.

import { knowledgeUri } from "./uri/uri.js";

const realApi = { ...(await import("./api.js")) }; // a copy: `mock.module` patches a loaded module in place
const realKnowledge = { ...(await import("./knowledge.js")) }; // a copy: `mock.module` patches a loaded module in place
let reads = 0;
let title = "Local Snapshots";
const listeners: Array<(e: Record<string, unknown>) => void> = [];
mock.module("./knowledge.js", () => ({
  ...realKnowledge,
  readKnowledgePages: async () => {
    reads++;
    return {
      pages: [{ uri: "oxplow:oxplow/knowledge?id=local-snapshots&store=oxwiki", title }],
      reads: { datasets: ["v_knowledge_page"], tables: [], measures: [] },
    };
  },
}));
mock.module("./api.js", () => ({
  ...realApi,
  subscribeOxplowEvents: (l: (e: Record<string, unknown>) => void) => {
    listeners.push(l);
    return () => listeners.splice(listeners.indexOf(l), 1);
  },
}));
// A fresh instance of the module under test: it keeps its map, its read and its
// event subscription for the life of the process, so one an earlier file used
// (a Markdown render reads the page list through the real api) is already
// subscribed to that file's events, not this file's.
const { useKnowledgeTitle } = (await import("./knowledgeTitleCache.js?fresh")) as typeof import("./knowledgeTitleCache.js");

function Title() {
  return <span data-testid="t">{useKnowledgeTitle(knowledgeUri("oxwiki", "local-snapshots")) ?? "…"}</span>;
}

const changed = { kind: "datasetsChanged", datasets: ["v_knowledge_page"] };

test("a burst of knowledge changes is one re-read, and the last change is seen", async () => {
  const view = render(<Title />);
  await waitFor(() => expect(view.getByTestId("t").textContent).toBe("Local Snapshots"));
  expect(reads).toBe(1);

  // Commits arrive one by one, each a little after the last.
  title = "Snapshots";
  for (let i = 0; i < 10; i++) {
    act(() => listeners.forEach((l) => l(changed)));
    await new Promise((r) => setTimeout(r, 20));
  }
  await waitFor(() => expect(view.getByTestId("t").textContent).toBe("Snapshots"));
  await new Promise((r) => setTimeout(r, 250));
  expect(reads).toBe(2);
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("./knowledge.js", () => realKnowledge);
  mock.module("./api.js", () => realApi);
});
