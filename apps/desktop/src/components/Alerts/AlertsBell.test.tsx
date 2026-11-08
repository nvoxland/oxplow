import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, within, fireEvent, render, waitFor } from "@testing-library/react";

// Everything that needs the person is counted by the dock's Alerts icon and
// announced by a toast once; the Alerts page itself is a foundation page.

const realApi = { ...(await import("../../api.js")) }; // a copy: `mock.module` patches a loaded module in place
/** Any other read (the Go To section's wiki pages, …): no rows. Never the
 *  real call — with no Tauri host it throws, and the test would pass only
 *  where another file's mock answered first (tsk1012). */
const noRows = { columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} };
let extensionRows: unknown[][] = [];
/** The failed operations `v_op_error` answers with. */
let opErrorRows: unknown[][] = [];
const landedHint = [4, "oxplow-bundled/landed-in-progress", "“t” is still in progress, but a commit landed its work.", "Main"];
/** The hints `v_agent_nudge` answers with. */
let hintRows: unknown[][] = [landedHint];
const pushFailed = ["e-push", "2026-10-22T10:00:00Z", "Push to origin", "git push", null, 1, null, 40, 12, 0];
const saved: unknown[] = [];
const ran: [string, unknown][] = [];
mock.module("../../api.js", () => ({
  ...realApi,
  getPanelLayout: async () => [],
  setPanelLayout: async (layout: unknown) => {
    saved.push(layout);
  },
  listExtensions: async () => [],
  listCommentsForStream: async () => [],
  subscribeCommentEvents: () => () => {},
  subscribeOxplowEvents: () => () => {},
  readEventContent: async (id: string, body: string) =>
    id === "e-push" && body === "output" ? { text: JSON.stringify({ stderr: "rejected" }), size: 20, truncated: false } : null,
  runCommand: async (name: string, input: unknown) => {
    ran.push([name, input]);
    return { result: null, audit_id: 1, event_id: "e", inverse: null };
  },
  querySql: async (sql: string) => {
    if (sql.includes("FROM v_extension")) {
      return {
        columns: ["name", "enabled", "errors", "deployment"],
        rows: extensionRows,
        truncated: false,
        reads: { datasets: ["v_extension"], tables: [], measures: [] },
        freshness: {},
      };
    }
    if (sql.includes("FROM v_op_error")) {
      return {
        columns: ["event_id", "at", "label", "command", "message", "exit_code", "signal", "duration_ms", "output_size", "seen"],
        rows: opErrorRows,
        truncated: false,
        reads: { datasets: ["v_op_error"], tables: [], measures: [] },
        freshness: {},
      };
    }
    if (sql.includes("FROM v_agent_nudge")) {
      return {
        columns: ["id", "kind", "message", "thread_title"],
        rows: hintRows,
        truncated: false,
        reads: { datasets: ["v_agent_nudge"], tables: [], measures: [] },
        freshness: {},
      };
    }
    if (sql.includes("FROM v_command_proposal")) {
      return {
        columns: ["id", "uri", "created_at", "command", "input", "actor_kind", "actor_id", "thread_id", "key", "preview", "dry_run"],
        rows: [[7, "oxplow:oxplow/proposal?id=7", "t", "oxplow.config.set", '{"key":"zones","value":[]}', "agent", "thr3", 3, "oxplow:oxplow/config?key=zones", '{"command":"oxplow.config.set","summary":"Set zones","input":{},"destructive":false}', null]],
        truncated: false,
        reads: { datasets: ["v_command_proposal"], tables: [], measures: [] },
        freshness: {},
      };
    }
    if (sql.includes("FROM v_event_dead_letter")) {
      return {
        columns: ["id", "consumer", "event_seq", "event_type", "error", "attempts", "last_failed_at"],
        rows: [
          [8, "change.analyze", 41, "snapshot.taken", "boom", 1, "t"],
          [9, "contribution.repair", 42, "contribution.disabled", "boom", 1, "t"],
        ],
        truncated: false,
        reads: { datasets: ["v_event_dead_letter"], tables: [], measures: [] },
        freshness: {},
      };
    }
    return noRows;
  },
}));
const { Dock } = await import("../Dock/Dock.js");
const { dockStore } = await import("../Dock/dockStore.js");
const { PanelRunsProvider } = await import("../Panels/PanelRunsContext.js");
const { alertsRef } = await import("../../tabs/pageRefs.js");
const { getToastStore } = await import("../toastStore.js");
const { useAlertToasts, SETTLE_MS } = await import("./useAlertToasts.js");
const { renderHook } = await import("@testing-library/react");
const { createInlineClaims } = await import("../../commands/actionFailures.js");

const withRuns = (node: React.ReactNode) => (
  <PanelRunsProvider streamId={null} threadId={null}>
    {node}
  </PanelRunsProvider>
);

afterEach(() => {
  saved.length = 0;
  ran.length = 0;
  opErrorRows = [];
  hintRows = [landedHint];
  dockStore.close();
  cleanup();
});

test("the dock's Alerts icon counts everything, red for a problem, and opens the panel", async () => {
  opErrorRows = [pushFailed];
  const opened: unknown[] = [];
  const view = render(withRuns(<Dock streamId={null} onOpenPage={(r) => opened.push(r)} />));
  // One proposal, one hint, two undelivered events, one failed operation.
  await waitFor(() => expect(view.getByTestId("dock-alerts-badge").textContent).toBe("5"));
  expect(view.getByTestId("dock-alerts-badge").getAttribute("data-tone")).toBe("danger");
  fireEvent.click(view.getByTestId("dock-alerts"));
  expect(view.getByTestId("dock-panel-alerts")).toBeTruthy();
  await waitFor(() => expect(view.getAllByTestId("alerts-panel-item").length).toBeGreaterThan(0));
  fireEvent.click(view.getByTestId("dock-panel-open-page").querySelector("[data-testid=open-page-link]")!);
  expect(opened).toEqual([alertsRef()]);
});

// Each row leads with the open arrow (the icon that opens a thing's page,
// here the item's), named Review; the message follows it.
test("a row's Review is the open arrow at its left and opens the item's page", async () => {
  const opened: unknown[] = [];
  const view = render(withRuns(<Dock streamId={null} onOpenPage={(r) => opened.push(r)} />));
  fireEvent.click(view.getByTestId("dock-alerts"));
  const row = await waitFor(() => view.getAllByTestId("alerts-panel-item")[0]!);
  const review = row.firstElementChild!;
  expect(review.getAttribute("data-testid")).toBe("alerts-panel-review");
  expect(review.getAttribute("aria-label")).toBe("Review");
  expect(review.querySelector("svg")).not.toBeNull();
  fireEvent.click(review);
  expect(opened).toEqual([alertsRef()]);
});

// Clear is a button at the panel's foot. It puts away what a person can put
// away: the hints and the failed operations. A decision is made, not
// cleared, and an undelivered event is retried or discarded on the page.
test("Clear dismisses the hints and failed operations and leaves the decisions", async () => {
  opErrorRows = [pushFailed];
  const view = render(withRuns(<Dock streamId={null} onOpenPage={() => {}} />));
  fireEvent.click(view.getByTestId("dock-alerts"));
  const clear = await waitFor(() => view.getByTestId("alerts-panel-clear"));
  expect(clear.className).toContain("ox-button");
  fireEvent.click(clear);
  await waitFor(() => expect(ran).toContainEqual(["oxplow.op_error.dismiss_all", {}]));
  expect(ran).toContainEqual(["oxplow.hint.dismiss_all", {}]);
  expect(ran.map(([name]) => name).sort()).toEqual(["oxplow.hint.dismiss_all", "oxplow.op_error.dismiss_all"]);
});

test("Clear is absent while only decisions and deliveries wait", async () => {
  hintRows = [];
  const view = render(withRuns(<Dock streamId={null} onOpenPage={() => {}} />));
  fireEvent.click(view.getByTestId("dock-alerts"));
  await waitFor(() => expect(view.getAllByTestId("alerts-panel-item").length).toBe(2));
  expect(view.queryByTestId("alerts-panel-clear")).toBeNull();
});

test("a toast once per new item, after the start-up settle, offering only Review", () => {
  let clock = 0;
  const none = { proposals: [], opErrors: [], undelivered: 0, failedReactions: 0, extensionProblems: [], badges: [], hints: [] };
  const reviewed: number[] = [];
  const { rerender } = renderHook(
    ({ items }) => useAlertToasts(items, () => reviewed.push(1), () => clock),
    { initialProps: { items: { ...none, proposals: [{ id: "oxplow:oxplow/proposal?id=1", title: "Already waiting" }] } } },
  );
  const before = getToastStore().getSnapshot().length;
  clock = SETTLE_MS + 1;
  const items = { ...none, proposals: [{ id: "oxplow:oxplow/proposal?id=1", title: "Already waiting" }, { id: "oxplow:oxplow/proposal?id=2", title: "New one" }] };
  rerender({ items });
  rerender({ items: { ...items } });
  const toasts = getToastStore().getSnapshot().slice(before);
  expect(toasts.map((t) => [t.message, t.actionLabel])).toEqual([["New one", "Review"]]);
  getToastStore().undo(toasts[0]!.id);
  expect(reviewed).toEqual([1]);
});

// A failure shown beside the control that ran it isn't toasted too; one
// whose control is gone is.
test("a failed operation shown beside its control gets no toast; one with no control does", async () => {
  let clock = 0;
  const none = { proposals: [], opErrors: [], undelivered: 0, failedReactions: 0, extensionProblems: [], badges: [], hints: [] };
  const claims = createInlineClaims((f) => void Promise.resolve().then(f), 0);
  claims.claim(Promise.resolve("e-inline"), () => true);
  const { rerender } = renderHook(({ items }) => useAlertToasts(items, () => {}, () => clock, claims), { initialProps: { items: none } });
  const before = getToastStore().getSnapshot().length;
  clock = SETTLE_MS + 1;
  rerender({ items: { ...none, opErrors: [{ id: "e-inline", label: "Approve" }, { id: "e-away", label: "Push to origin" }] } });
  await waitFor(() => expect(getToastStore().getSnapshot().length).toBe(before + 1));
  expect(getToastStore().getSnapshot().slice(before).map((t) => t.message)).toEqual(["Push to origin"]);
});

// A failure this window caused is new even as the window opens; one that
// was already recorded then is taken as seen.
test("in the first moments, only a failure this window's runner recorded is toasted", async () => {
  const none = { proposals: [], opErrors: [], undelivered: 0, failedReactions: 0, extensionProblems: [], badges: [], hints: [] };
  const claims = createInlineClaims((f) => void Promise.resolve().then(f), 0);
  claims.claim(Promise.resolve("e-mine"), null);
  const before = getToastStore().getSnapshot().length;
  renderHook(() =>
    useAlertToasts({ ...none, opErrors: [{ id: "e-old", label: "Old push" }, { id: "e-mine", label: "Boom" }] }, () => {}, () => 0, claims),
  );
  await waitFor(() => expect(getToastStore().getSnapshot().length).toBe(before + 1));
  expect(getToastStore().getSnapshot().slice(before).map((t) => t.message)).toEqual(["Boom"]);
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
