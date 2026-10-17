import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor, within } from "@testing-library/react";

import type { Thread } from "../api.js";
import type { EarlierSessions, HarnessListing } from "../tauri-bridge/generated/bindings.js";
import type { EarlierRow } from "../components/earlierSessionRows.js";
import { NewSessionPage } from "./NewSessionPage.js";

afterEach(cleanup);

const thread = { id: "thr1", title: "Fix the cart" } as unknown as Thread;
const harnesses: HarnessListing[] = [
  { id: "claude", title: "Claude", enabled: true, settings: [] },
  { id: "codex", title: "Codex", enabled: true, settings: [] },
];

function renderPicker() {
  const started: Array<[string, boolean]> = [];
  const declined: boolean[] = [];
  const view = render(
    <NewSessionPage
      thread={thread}
      harnesses={harnesses}
      openSessions={0}
      onStart={async (harness, remember) => {
        started.push([harness, remember]);
      }}
      onNoSession={async (remember) => {
        declined.push(remember);
      }}
    />,
  );
  return { ...view, started, declined };
}

/** The heading says what the thread has: none yet (a new thread), or how
 *  many are open (the launcher's New Agent Session…). */
test("the heading counts the thread's open sessions", () => {
  const none = render(<NewSessionPage thread={thread} harnesses={harnesses} openSessions={0} onStart={async () => {}} onNoSession={async () => {}} />);
  expect(none.getByTestId("new-session-empty").textContent).toContain("“Fix the cart” has no agent running here yet");
  expect(none.queryByTestId("new-session-none")).not.toBeNull();
  cleanup();
  const two = render(<NewSessionPage thread={thread} harnesses={harnesses} openSessions={2} onStart={async () => {}} onNoSession={async () => {}} />);
  const heading = two.getByTestId("new-session-empty").textContent ?? "";
  expect(heading).toContain("“Fix the cart” has 2 agent sessions open");
  expect(heading).not.toContain("no agent");
  // Declining a session only makes sense for a thread without one.
  expect(two.queryByTestId("new-session-none")).toBeNull();
});

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
  await waitFor(() => expect(started).toEqual([["codex", true]]));
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

// The earlier sessions: closed here, resumable, and one open elsewhere.
const earlier: EarlierSessions = {
  rows: [
    {
      harness: "claude",
      session: { session: "agent_session:ses3", display_name: "The parser", thread: "thread:thr1", thread_title: "Fix the cart", here: true, open: false, closed_at: "2026-10-10T11:00:00Z", turns: 4, last_prompt: "ship it" },
      conversation: { id: "c1", title: "The parser", first_prompt: null, started_at: null, updated_at: "2026-10-10T11:00:00Z", size_bytes: null, parent: null },
      action: "reopen",
    },
    {
      harness: "codex",
      session: null,
      conversation: { id: "c2", title: "Billing refund", first_prompt: null, started_at: null, updated_at: "2026-10-09T09:00:00Z", size_bytes: 2048, parent: null },
      action: "resume",
    },
    {
      harness: "claude",
      session: { session: "agent_session:ses9", display_name: "Helper", thread: "thread:thr2", thread_title: "Other", here: false, open: true, closed_at: null, turns: 1, last_prompt: null },
      conversation: { id: "c3", title: "Busy elsewhere", first_prompt: null, started_at: null, updated_at: "2026-10-08T09:00:00Z", size_bytes: null, parent: null },
      action: "none",
    },
  ],
  unavailable: [{ harness: "codex", reason: "its daemon isn't running" }],
};

function renderEarlier(listing: EarlierSessions = earlier) {
  const taken: EarlierRow[] = [];
  const view = render(
    <NewSessionPage
      thread={thread}
      harnesses={harnesses}
      openSessions={0}
      onStart={async () => {}}
      onNoSession={async () => {}}
      onTakeEarlier={async (row) => {
        taken.push(row);
      }}
      loadEarlier={async () => listing}
    />,
  );
  return { ...view, taken };
}

test("earlier sessions list under the Start row, newest first, with a line for a harness that couldn't answer", async () => {
  const { getByTestId, findByTestId, getByRole } = renderEarlier();
  await findByTestId("earlier-row-agent_session:ses3");
  expect(within(getByRole("listbox")).getAllByRole("option").map((o) => o.getAttribute("data-action"))).toEqual(["reopen", "resume", "none"]);
  expect(getByTestId("earlier-row-agent_session:ses3").textContent).toContain("closed here");
  expect(getByTestId("earlier-row-codex:c2").textContent).toContain("not started in oxplow");
  expect(getByTestId("new-session-earlier-unavailable-codex").textContent).toContain("its daemon isn't running");
  // A conversation open in another thread's session can't be taken.
  expect((getByTestId("earlier-row-agent_session:ses9") as HTMLButtonElement).disabled).toBe(true);
});

test("the filter narrows the rows; ↓ and Enter take the row under the cursor; Escape clears the filter", async () => {
  const { findByTestId, getByTestId, queryByTestId, taken } = renderEarlier();
  const filter = (await findByTestId("new-session-earlier-filter")) as HTMLInputElement;
  fireEvent.change(filter, { target: { value: "billing" } });
  expect(queryByTestId("earlier-row-agent_session:ses3")).toBeNull();
  expect(getByTestId("earlier-row-codex:c2")).toBeTruthy();
  fireEvent.keyDown(filter, { key: "Escape" });
  expect(filter.value).toBe("");
  expect(getByTestId("earlier-row-agent_session:ses3")).toBeTruthy();

  fireEvent.keyDown(filter, { key: "ArrowDown" });
  fireEvent.keyDown(filter, { key: "Enter" });
  await waitFor(() => expect(taken.map((r) => [r.action, r.conversation])).toEqual([["resume", "c2"]]));
  fireEvent.keyDown(filter, { key: "ArrowUp" });
  fireEvent.keyDown(filter, { key: "Enter" });
  await waitFor(() => expect(taken.map((r) => r.action)).toEqual(["resume", "reopen"]));
});

test("clicking a row takes it, and a refusal shows on the page", async () => {
  const view = render(
    <NewSessionPage
      thread={thread}
      harnesses={harnesses}
      openSessions={0}
      onStart={async () => {}}
      onNoSession={async () => {}}
      onTakeEarlier={async () => {
        throw new Error("the session is open");
      }}
      loadEarlier={async () => earlier}
    />,
  );
  fireEvent.click(await view.findByTestId("earlier-row-agent_session:ses3"));
  await waitFor(() => expect(view.getByTestId("new-session-error").textContent).toContain("the session is open"));
});

test("with no earlier sessions the list says so", async () => {
  const { findByTestId } = renderEarlier({ rows: [], unavailable: [] });
  expect((await findByTestId("new-session-earlier-empty")).textContent).toContain("No earlier sessions");
});
