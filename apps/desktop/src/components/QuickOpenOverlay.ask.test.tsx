import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Stream } from "../api.js";
import { subscribeAgentInput } from "../agent-input-bus.js";

// The launcher's Ask entry (P6.D1): typed text becomes "Ask the Agent",
// put in the agent's input. Nothing is ever sent.

const realApi = await import("../api.js");
const ran: Array<[string, unknown, boolean]> = [];
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [
    {
      name: "x",
      enabled: true,
      ui: { slots: [], commands: [], decorators: [] },
      lenses: [],
      pages: [],
      panels: [],
    },
  ],
  runCommand: async (name: string, input: unknown, confirmed = false) => {
    ran.push([name, input, confirmed]);
    return { result: null, audit_id: 1, event_id: null, undo: null };
  },
}));

const { QuickOpenOverlay } = await import("./QuickOpenOverlay.js");

const stream = { id: "str1", title: "oxplow" } as unknown as Stream;
const inserted: string[] = [];
let off: () => void = () => {};

beforeEach(() => {
  inserted.length = 0;
  ran.length = 0;
  off = subscribeAgentInput((t) => inserted.push(t));
});
afterEach(() => {
  off();
  cleanup();
});

function renderOverlay(onClose = () => {}) {
  return render(
    <QuickOpenOverlay
      open
      stream={stream}
      threadId="thr1"
      selectedFilePath={null}
      pages={[]}
      offers={[]}
      onClose={onClose}
      onOpenFile={() => {}}
      onOpenPage={() => {}}
      onOpenSearchHit={() => {}}
    />,
  );
}

test("typed text offers Ask the Agent, which fills the agent's input", async () => {
  let closed = 0;
  const view = renderOverlay(() => closed++);
  const input = view.getByPlaceholderText(/Search everything/);
  fireEvent.change(input, { target: { value: "why is it slow" } });
  const ask = await waitFor(() => view.getByTestId("launcher-ask"));
  expect(ask.textContent).toContain("Ask the Agent: why is it slow");
  fireEvent.click(ask);
  expect(inserted).toEqual(["why is it slow"]);
  expect(closed).toBe(1);
});
