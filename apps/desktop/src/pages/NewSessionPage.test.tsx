import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Thread } from "../api.js";
import type { HarnessListing } from "../tauri-bridge/generated/bindings.js";
import { NewSessionPage } from "./NewSessionPage.js";

afterEach(cleanup);

const thread = { id: "thr1", title: "Fix the cart" } as unknown as Thread;
const harnesses: HarnessListing[] = [
  { id: "claude", title: "Claude", chat: false, enabled: true, settings: [] },
  { id: "codex", title: "Codex", chat: false, enabled: true, settings: [] },
];

function renderPicker() {
  const started: Array<[string, string | null, boolean]> = [];
  const declined: boolean[] = [];
  const view = render(
    <NewSessionPage
      thread={thread}
      harnesses={harnesses}
      onStart={async (harness, acpAgent, remember) => {
        started.push([harness, acpAgent, remember]);
      }}
      onNoSession={async (remember) => {
        declined.push(remember);
      }}
    />,
  );
  return { ...view, started, declined };
}

/** No Session in This Thread leaves the thread without one. */
test("no session in this thread declines, remembering only when asked", async () => {
  const first = renderPicker();
  fireEvent.click(first.getByTestId("new-session-none"));
  await waitFor(() => expect(first.declined).toEqual([false]));
  cleanup();

  const second = renderPicker();
  fireEvent.click(second.getByTestId("new-session-remember"));
  fireEvent.click(second.getByTestId("new-session-none"));
  await waitFor(() => expect(second.declined).toEqual([true]));
  expect(second.started).toEqual([]);
});

/** Start with Remember This makes the chosen agent what new threads start. */
test("start passes whether to remember the choice", async () => {
  const { getByTestId, started } = renderPicker();
  fireEvent.change(getByTestId("new-session-agent"), { target: { value: "codex" } });
  fireEvent.click(getByTestId("new-session-remember"));
  fireEvent.click(getByTestId("new-session-start"));
  await waitFor(() => expect(started).toEqual([["codex", null, true]]));
});

/** The page focuses its picker when it opens, but a page that opens late
 *  doesn't take focus from where a person already is (the launcher). */
test("the picker takes focus only when nothing else has it", () => {
  const first = renderPicker();
  expect(document.activeElement).toBe(first.getByTestId("new-session-agent"));
  cleanup();

  const input = document.createElement("input");
  document.body.appendChild(input);
  input.focus();
  renderPicker();
  expect(document.activeElement).toBe(input);
  input.remove();
});
