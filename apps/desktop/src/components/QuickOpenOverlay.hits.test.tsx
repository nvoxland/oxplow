import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Stream } from "../api.js";

// A search hit's row names what it is: the kind's icon and a person's word
// for it, for the core kinds as for an extension's.

const KINDS: Array<[string, string]> = [
  ["thread", "thread"],
  ["agent_session", "agent session"],
  ["turn", "turn"],
  ["request", "request"],
  ["commit", "commit"],
  ["dashboard", "dashboard"],
];

const realApi = { ...(await import("../api.js")) }; // a copy: `mock.module` patches a loaded module in place
const searched: Array<[string, string | null]> = [];
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  searchSite: async (query: string, streamId: string | null) => {
    searched.push([query, streamId]);
    if (query.startsWith("q-")) {
      return [{ uri: `oxplow:oxplow/thread?id=${query}`, resource: "oxplow/thread", title: query, snippet: "", stream_id: null, tier: 0, score: 0 }];
    }
    return KINDS.map(([kind]) => ({
      uri: `oxplow:oxplow/${kind}?id=x1`,
      resource: `oxplow/${kind}`,
      title: `A ${kind}`,
      snippet: "found it",
      stream_id: null,
      tier: 5,
      score: 1,
    }));
  },
}));

const { QuickOpenOverlay } = await import("./QuickOpenOverlay.js");

afterEach(cleanup);

// The icon says what a row is; its word is the icon's hover text, never a
// label at the row's end.
test("each hit row shows its kind's icon, named by its hover text", async () => {
  const view = render(
    <QuickOpenOverlay
      open
      stream={{ id: "str1", title: "oxplow" } as unknown as Stream}
      threadId="thr1"
      selectedFilePath={null}
      offers={[]}
      onClose={() => {}}
      onOpenFile={() => {}}
      onOpenPage={() => {}}
      onOpenSearchHit={() => {}}
    />,
  );
  fireEvent.change(view.getByPlaceholderText(/Search everything/), { target: { value: "found" } });
  for (const [kind, label] of KINDS) {
    const row = await waitFor(() => view.getByTestId(`launcher-hit-oxplow:oxplow/${kind}?id=x1`));
    expect(row.querySelector("svg"), `${kind} icon`).not.toBeNull();
    expect(row.querySelector(`[title="${label}"]`), `${kind} hover text`).not.toBeNull();
    expect(row.textContent, kind).toMatch(/found it$/);
  }
});

// Files are the current stream's: the search is asked for its stream,
// from the first character.
test("the search is the current stream's", async () => {
  searched.length = 0;
  const view = render(
    <QuickOpenOverlay
      open
      stream={{ id: "str1", title: "oxplow" } as unknown as Stream}
      threadId="thr1"
      selectedFilePath={null}
      offers={[]}
      onClose={() => {}}
      onOpenFile={() => {}}
      onOpenPage={() => {}}
      onOpenSearchHit={() => {}}
    />,
  );
  fireEvent.change(view.getByPlaceholderText(/Search everything/), { target: { value: "d" } });
  await waitFor(() => expect(searched).toEqual([["d", "str1"]]));
});

// Enter takes the first hit for what was typed, never one still showing
// for an earlier query.
test("Enter before the search answers takes the answer's first hit", async () => {
  const opened: string[] = [];
  const view = render(
    <QuickOpenOverlay
      open
      stream={{ id: "str1", title: "oxplow" } as unknown as Stream}
      threadId="thr1"
      selectedFilePath={null}
      offers={[]}
      onClose={() => {}}
      onOpenFile={() => {}}
      onOpenPage={() => {}}
      onOpenSearchHit={(hit) => opened.push(hit.uri)}
    />,
  );
  const input = view.getByPlaceholderText(/Search everything/);
  fireEvent.change(input, { target: { value: "q-a" } });
  await waitFor(() => view.getByTestId("launcher-hit-oxplow:oxplow/thread?id=q-a"));
  fireEvent.change(input, { target: { value: "q-ab" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(opened).toEqual(["oxplow:oxplow/thread?id=q-ab"]));
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../api.js", () => realApi);
});
