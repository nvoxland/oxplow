import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Stream } from "../api.js";
import { subscribeAgentInput } from "../agent-input-bus.js";

// The launcher's Ask entries (P6.D1): typed text becomes "Ask the Agent",
// an extension's prompt entry is put in the agent's input, and its
// command entry runs as the person. Nothing is ever sent.

const realApi = await import("../api.js");
const ran: Array<[string, unknown, boolean]> = [];
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [
    {
      name: "x",
      enabled: true,
      lenses: [],
      pages: [],
      panels: [],
      launcher: [
        { label: "Ask why slow", category: "Code", target: { kind: "prompt", prompt: "Why is the build slow?" } },
        { label: "File a bug", category: "Work", target: { kind: "command", command: "work_item.create", input: { title: "Bug" } } },
      ],
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
      menuGroups={[]}
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

test("an extension's prompt entry fills the input; its command entry runs as the person", async () => {
  const view = renderOverlay();
  const input = view.getByPlaceholderText(/Search everything/);
  fireEvent.change(input, { target: { value: "why slow" } });
  // A launcher action runs after the overlay closes.
  fireEvent.click(await waitFor(() => view.getByText("Ask why slow")));
  await waitFor(() => expect(inserted).toEqual(["Why is the build slow?"]));

  fireEvent.change(input, { target: { value: "file a bug" } });
  fireEvent.click(await waitFor(() => view.getByText("File a bug")));
  await waitFor(() => expect(ran).toEqual([["work_item.create", { title: "Bug" }, false]]));
});
