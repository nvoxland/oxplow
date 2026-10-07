import { expect, mock, test } from "bun:test";
import { act, render, waitFor } from "@testing-library/react";

// A burst of commits touching the wiki (a scan storing its findings
// writes page refs) is one re-read of the page list, not one per commit —
// and a change while a read is running is still read afterwards.

const realApi = await import("./api.js");
const realKnowledge = await import("./knowledge.js");
let reads = 0;
let title = "Local Snapshots";
const listeners: Array<(e: Record<string, unknown>) => void> = [];
mock.module("./knowledge.js", () => ({
  ...realKnowledge,
  readWikiPages: async () => {
    reads++;
    return {
      pages: [{ slug: "local-snapshots", title }],
      reads: { models: ["v_knowledge_page"], tables: [], measures: [] },
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
const { useWikiTitle } = await import("./wikiTitleCache.js");

function Title() {
  return <span data-testid="t">{useWikiTitle("local-snapshots") ?? "…"}</span>;
}

const changed = { kind: "modelsChanged", models: ["v_knowledge_page"] };

test("a burst of wiki changes is one re-read, and the last change is seen", async () => {
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
