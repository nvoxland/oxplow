import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Stream } from "../api.js";

// A search hit's row names what it is: the kind's icon and a person's word
// for it, for the core kinds as for an extension's.

const KINDS: Array<[string, string]> = [
  ["thread", "thread"],
  ["agent_session", "agent session"],
  ["turn", "turn"],
  ["effort", "effort"],
  ["decision", "decision"],
  ["commit", "commit"],
  ["dashboard", "dashboard"],
  ["lens", "lens"],
  ["thread_note", "note"],
];

const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  searchSite: async () =>
    KINDS.map(([kind]) => ({
      uri: `oxplow:oxplow/${kind}?id=x1`,
      resource: `oxplow/${kind}`,
      title: `A ${kind}`,
      snippet: "found it",
      stream_id: null,
      score: 1,
    })),
}));

const { QuickOpenOverlay } = await import("./QuickOpenOverlay.js");

afterEach(cleanup);

test("each hit row shows its kind's icon and label", async () => {
  const view = render(
    <QuickOpenOverlay
      open
      stream={{ id: "str1", title: "oxplow" } as unknown as Stream}
      threadId="thr1"
      selectedFilePath={null}
      pages={[]}
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
    expect(row.textContent, kind).toContain(`found it${label}`);
  }
});
