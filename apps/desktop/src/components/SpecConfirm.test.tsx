import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// tsk898: a git action asks first exactly when its command's spec does.

const realApi = await import("../api.js");
const confirms: Record<string, string> = { "vcs.push": "never", "vcs.merge": "destructive" };
mock.module("../api.js", () => ({
  ...realApi,
  getCommand: async (name: string) => ({ name, summary: name, confirm: confirms[name] }),
}));

const { SpecConfirm } = await import("./SpecConfirm.js");

afterEach(cleanup);

function button(command: string, ran: string[]) {
  return render(
    <SpecConfirm command={command} onConfirm={() => ran.push(command)} confirmLabel="Go" testIdPrefix="t">
      {(run) => (
        <button type="button" data-testid="action" onClick={run}>
          Act
        </button>
      )}
    </SpecConfirm>,
  );
}

test("a command that never asks runs on the click", async () => {
  const ran: string[] = [];
  const view = button("vcs.push", ran);
  // Until the spec loads it asks; once it says `never`, the click runs.
  await waitFor(() => {
    fireEvent.click(view.getByTestId("action"));
    expect(ran).toEqual(["vcs.push"]);
  });
});

test("a command that asks arms on the first click and runs on the confirm", async () => {
  const ran: string[] = [];
  const view = button("vcs.merge", ran);
  await new Promise((r) => setTimeout(r, 20));
  fireEvent.click(view.getByTestId("action"));
  expect(ran).toEqual([]);
  fireEvent.click(view.getByTestId("t-confirm"));
  expect(ran).toEqual(["vcs.merge"]);
});
