import { expect, mock, test } from "bun:test";
import { act, render, waitFor } from "@testing-library/react";

// A burst of `v_change` commits (an analysis landing, its duplicates
// stored) asks for the change once, after the burst — not once per commit.

const realApi = await import("../api.js");
let ensures = 0;
const listeners: Array<(e: Record<string, unknown>) => void> = [];
mock.module("../api.js", () => ({
  ...realApi,
  ensureChange: async () => {
    ensures++;
    return { id: 5 };
  },
  subscribeOxplowEvents: (l: (e: Record<string, unknown>) => void) => {
    listeners.push(l);
    return () => listeners.splice(listeners.indexOf(l), 1);
  },
}));
const { useChange } = await import("./useChange.js");

function Change() {
  const { change } = useChange({ kind: "working", streamId: "str1" } as never);
  return <span data-testid="c">{change ? String(change.id) : "…"}</span>;
}

test("a burst of v_change commits re-ensures the change once", async () => {
  const view = render(<Change />);
  await waitFor(() => expect(view.getByTestId("c").textContent).toBe("5"));
  expect(ensures).toBe(1);
  for (let i = 0; i < 10; i++) {
    act(() => listeners.forEach((l) => l({ kind: "modelsChanged", models: ["v_change"] })));
    await new Promise((r) => setTimeout(r, 20));
  }
  await new Promise((r) => setTimeout(r, 250));
  expect(ensures).toBe(2);
});
