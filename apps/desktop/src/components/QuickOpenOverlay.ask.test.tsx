import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Stream } from "../api.js";

// The launcher's Ask entry (P6.D1): typed text becomes "Ask the Assistant",
// put in the Assistant's box. Nothing is ever sent.

const realApi = { ...(await import("../api.js")) }; // a copy: `mock.module` patches a loaded module in place
const ran: Array<[string, unknown, boolean]> = [];
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [
    {
      name: "x",
      enabled: true,
      ui: { slots: [], commands: [], decorators: [] },
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
const { getAssistant, resetAssistant } = await import("./Assistant/assistantSession.js");
const { dockStore } = await import("./Dock/dockStore.js");

beforeEach(() => {
  ran.length = 0;
});
afterEach(() => {
  cleanup();
  resetAssistant();
  dockStore.close();
});

function renderOverlay(onClose = () => {}) {
  return render(
    <QuickOpenOverlay
      open
      stream={stream}
      threadId="thr1"
      selectedFilePath={null}
      offers={[]}
      onClose={onClose}
      onOpenFile={() => {}}
      onOpenPage={() => {}}
      onOpenSearchHit={() => {}}
    />,
  );
}

test("typed text offers Ask the Assistant, which fills its box", async () => {
  let closed = 0;
  const view = renderOverlay(() => closed++);
  const input = view.getByPlaceholderText(/Search everything/);
  fireEvent.change(input, { target: { value: "why is it slow" } });
  const ask = await waitFor(() => view.getByTestId("launcher-ask"));
  expect(ask.textContent).toContain("Ask the Assistant: why is it slow");
  fireEvent.click(ask);
  expect(getAssistant().draft?.text).toBe("why is it slow");
  expect(dockStore.get().open).toBe("assistant");
  expect(closed).toBe(1);
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../api.js", () => realApi);
});
