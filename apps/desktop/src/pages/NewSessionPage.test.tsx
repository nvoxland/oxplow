import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor, within } from "@testing-library/react";

import type { Thread } from "../api.js";
import type { EarlierSessions, HarnessListing } from "../tauri-bridge/generated/bindings.js";
import type { EarlierRow } from "../components/earlierSessionRows.js";
import { createRememberedStore, RememberedContext } from "../prefs/remembered.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { pageTabRef } from "../tabs/pageRefs.js";
import { NewSessionPage } from "./NewSessionPage.js";

afterEach(cleanup);

const thread = { id: "thr1", title: "Fix the cart" } as unknown as Thread;
const harnesses: HarnessListing[] = [
  { id: "claude", title: "Claude", enabled: true, settings: [] },
  { id: "codex", title: "Codex", enabled: true, settings: [] },
];

function renderPicker() {
  const started: Array<[string, boolean]> = [];
  const view = render(
    <NewSessionPage
      thread={thread}
      harnesses={harnesses}
      onStart={async (harness, remember) => {
        started.push([harness, remember]);
      }}
    />,
  );
  return { ...view, started };
}

/** The page is the agent picker and the earlier sessions: it says nothing
 *  of the thread's sessions (its tabs show them) and offers no way to
 *  decline one (the tab's × closes it); the button is New Session. */
test("the page offers New Session, with nothing about the thread's sessions", () => {
  const { getByTestId, queryByTestId, container } = renderPicker();
  expect(getByTestId("new-session-start").textContent).toBe("New Session");
  expect(queryByTestId("new-session-empty")).toBeNull();
  expect(queryByTestId("new-session-none")).toBeNull();
  expect(container.textContent).not.toContain("no agent running");
  expect(container.textContent).not.toContain("Settings → Agents changes it");
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
      onStart={async () => {}}
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
  // Its status is an icon, its words the icon's tooltip.
  const status = getByTestId("earlier-row-agent_session:ses3").querySelector("[data-status]") as HTMLElement;
  expect(status.dataset.status).toBe("closed_here");
  expect(status.getAttribute("title")).toBe("closed here");
  expect(getByTestId("earlier-row-codex:c2").querySelector("[data-status]")?.getAttribute("title")).toBe("not started in oxplow");
  expect(getByTestId("new-session-earlier-unavailable-codex").textContent).toContain("its daemon isn't running");
  // A conversation open in another thread's session can't be taken.
  expect(getByTestId("earlier-row-agent_session:ses9").getAttribute("aria-disabled")).toBe("true");
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
      onStart={async () => {}}
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

/** One name everywhere: the heading says what the tab and launcher say. */
test("the page is headed New session", () => {
  const { container } = renderPicker();
  expect(container.querySelector("h1")?.textContent).toBe("New session");
});

const twoRows: EarlierSessions = {
  rows: [
    { harness: "claude", session: null, conversation: { id: "a", title: "Outside", first_prompt: null, started_at: null, updated_at: null, size_bytes: null, parent: null }, action: "resume" },
    {
      harness: "claude",
      session: { session: "agent_session:ses2", name: "Busy", conversation: null, display_name: "Busy", thread: "thread:thr2", thread_title: "Other", here: false, open: true, closed_at: null, turns: 0, last_prompt: null },
      conversation: null,
      action: "none",
    },
  ],
  unavailable: [],
} as unknown as EarlierSessions;

/** A cell with nothing to say shows a dash, so a blank never reads as a
 *  value that failed to load; the icons in use are explained once. */
test("empty cells show a dash, and the icons shown have a key", async () => {
  const { getByTestId } = render(
    <NewSessionPage thread={thread} harnesses={harnesses} onStart={async () => {}} loadEarlier={async () => twoRows} />,
  );
  const table = await waitFor(() => getByTestId("new-session-earlier-rows"));
  const outside = within(table).getByTestId("earlier-row-claude:a");
  expect(outside.textContent).toContain("—");
  const legend = getByTestId("new-session-earlier-legend");
  expect(legend.textContent).toContain("not started in oxplow");
  expect(legend.textContent).toContain("open in another thread");
  expect(legend.textContent).not.toContain("closed here");
});

/** The agent picked is the page's memory, kept for the person across tabs of it. */
test("the agent picked is page memory: a new tab of the page starts at it", () => {
  const store = createRememberedStore({ load: async () => [], save: async () => {} });
  const page = () => (
    <RememberedContext.Provider value={store}>
      <PageNavigationContext.Provider value={{ ref: pageTabRef("new-session") } as unknown as PageNavigation}>
        <NewSessionPage thread={thread} harnesses={harnesses} onStart={async () => {}} />
      </PageNavigationContext.Provider>
    </RememberedContext.Provider>
  );
  const first = render(page());
  fireEvent.change(first.getByTestId("new-session-agent"), { target: { value: "codex" } });
  expect(store.get("page:new-session", "agent")).toBe("codex");
  first.unmount();
  const second = render(page());
  expect((second.getByTestId("new-session-agent") as HTMLSelectElement).value).toBe("codex");
});

// The filter on the earlier sessions is the page's view (`q`): in the tab's
// address, so Back and a restart keep it; whether to remember the choice is
// an unsaved form, the step's history state.
test("the filter is the tab's `q`, and Remember This is the step's draft", async () => {
  const patches: Array<Record<string, unknown>> = [];
  const states: Array<Record<string, unknown>> = [];
  const nav = {
    ref: pageTabRef("new-session"),
    viewParams: { q: "billing" },
    patchViewParams: (p: Record<string, unknown>) => patches.push(p),
    historyState: { remember: true },
    setHistoryState: (p: Record<string, unknown>) => states.push(p),
  } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <NewSessionPage thread={thread} harnesses={harnesses} onStart={async () => {}} loadEarlier={async () => earlier} />
    </PageNavigationContext.Provider>,
  );
  const filter = (await view.findByTestId("new-session-earlier-filter")) as HTMLInputElement;
  expect(filter.value).toBe("billing");
  expect(view.queryByTestId("earlier-row-agent_session:ses3")).toBeNull();
  expect((view.getByTestId("new-session-remember") as HTMLInputElement).checked).toBe(true);
  fireEvent.change(filter, { target: { value: "parser" } });
  expect(patches).toEqual([{ q: "parser" }]);
  fireEvent.click(view.getByTestId("new-session-remember"));
  expect(states).toEqual([{ remember: false }]);
});
