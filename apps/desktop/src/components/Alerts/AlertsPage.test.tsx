import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, within, fireEvent, render, waitFor } from "@testing-library/react";

// Everything that needs the person is on the Alerts page,
// counted by the status bar's bell, and announced by a toast once.

const realApi = await import("../../api.js");
/** Any other read (the Go To section's wiki pages, …): no rows. Never the
 *  real call — with no Tauri host it throws, and the test would pass only
 *  where another file's mock answered first (tsk1012). */
const noRows = { columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} };
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
  runCommand: async (name: string, input: unknown) => {
    ran.push([name, input]);
    return { result: null, audit_id: 1, event_id: "e", inverse: null };
  },
  querySql: async (sql: string) => {
    if (sql.includes("FROM v_agent_nudge")) {
      return {
        columns: ["id", "kind", "message", "thread_title"],
        rows: [[4, "oxplow-bundled/landed-in-progress", "“t” is still in progress, but a commit landed its work.", "Main"]],
        truncated: false,
        reads: { models: ["v_agent_nudge"], tables: [], measures: [] },
        freshness: {},
      };
    }
    if (sql.includes("FROM v_command_proposal")) {
      return {
        columns: ["id", "ref", "created_at", "command", "input", "actor_kind", "actor_id", "thread_id", "key", "preview", "dry_run"],
        rows: [[7, "proposal:7", "t", "oxplow.config.set", '{"key":"zones","value":[]}', "agent", "thr3", 3, "config:zones", '{"command":"oxplow.config.set","summary":"Set zones","input":{},"destructive":false}', null]],
        truncated: false,
        reads: { models: ["v_command_proposal"], tables: [], measures: [] },
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
        reads: { models: ["v_event_dead_letter"], tables: [], measures: [] },
        freshness: {},
      };
    }
    return noRows;
  },
}));
const { AlertsPage } = await import("../../pages/AlertsPage.js");
const { AlertsIndicator } = await import("./AlertsIndicator.js");
const { PanelRunsProvider } = await import("../Panels/PanelRunsContext.js");
const { getOpErrorsStore, recordOpError } = await import("../opErrorsStore.js");
const { alertsRef } = await import("../../tabs/pageRefs.js");
const { getToastStore } = await import("../toastStore.js");
const { useAlertToasts, SETTLE_MS } = await import("./useAlertToasts.js");
const { renderHook } = await import("@testing-library/react");

const withRuns = (node: React.ReactNode) => (
  <PanelRunsProvider streamId={null} threadId={null}>
    {node}
  </PanelRunsProvider>
);

afterEach(() => {
  saved.length = 0;
  ran.length = 0;
  getOpErrorsStore().clear();
  cleanup();
});

test("the page lists a waiting proposal as its card, failed operations and undelivered events", async () => {
  recordOpError({ label: "Push to origin", stderr: "rejected" });
  const view = render(withRuns(<AlertsPage onOpenPage={() => {}} />));
  const decisions = await waitFor(() => view.getByTestId("alerts-decisions"));
  await waitFor(() => expect(within(decisions).getByTestId("proposal-7")).toBeTruthy());
  const problems = view.getByTestId("alerts-problems");
  await waitFor(() => expect(within(problems).getByTestId("delivery-row-8")).toBeTruthy());
  expect(within(problems).getByTestId("delivery-row-9")).toBeTruthy();
  const op = getOpErrorsStore().getSnapshot()[0]!;
  const row = within(problems).getByTestId(`alerts-op-${op.id}`);
  expect(view.queryByTestId(`op-error-detail-${op.id}`)).toBeNull();
  fireEvent.click(row);
  expect(view.getByTestId(`op-error-detail-${op.id}`).textContent).toContain("rejected");
});

test("a hint raised to the person is listed with its thread, and Dismiss dismisses it", async () => {
  const view = render(withRuns(<AlertsPage onOpenPage={() => {}} />));
  const hints = await waitFor(() => view.getByTestId("alerts-hints"));
  expect(within(hints).getByTestId("alerts-hint-4").textContent).toContain("“t” is still in progress");
  expect(within(hints).getByTestId("alerts-hint-4").textContent).toContain("Main");
  fireEvent.click(within(hints).getByTestId("alerts-hint-dismiss-4"));
  await waitFor(() => expect(ran).toEqual([["oxplow.hint.dismiss", { nudge: 4 }]]));
});

test("the bell counts everything, red for a problem, and opens Alerts", async () => {
  recordOpError({ label: "Push to origin", stderr: "rejected" });
  const opened: unknown[] = [];
  const view = render(withRuns(<AlertsIndicator onOpenPage={(r) => opened.push(r)} />));
  // One proposal, one hint, two undelivered events, one failed operation.
  await waitFor(() => expect(view.getByTestId("alerts-count").textContent).toBe("5"));
  expect(view.getByTestId("alerts-indicator").getAttribute("data-tone")).toBe("danger");
  fireEvent.click(view.getByTestId("alerts-indicator"));
  expect(opened).toEqual([alertsRef()]);
});

test("a toast once per new item, after the start-up settle, offering only Review", () => {
  let clock = 0;
  const none = { proposals: [], opErrors: [], undelivered: 0, failedReactions: 0, badges: [], hints: [] };
  const reviewed: number[] = [];
  const { rerender } = renderHook(
    ({ items }) => useAlertToasts(items, () => reviewed.push(1), () => clock),
    { initialProps: { items: { ...none, proposals: [{ id: "proposal:1", title: "Already waiting" }] } } },
  );
  const before = getToastStore().getSnapshot().length;
  clock = SETTLE_MS + 1;
  const items = { ...none, proposals: [{ id: "proposal:1", title: "Already waiting" }, { id: "proposal:2", title: "New one" }] };
  rerender({ items });
  rerender({ items: { ...items } });
  const toasts = getToastStore().getSnapshot().slice(before);
  expect(toasts.map((t) => [t.message, t.actionLabel])).toEqual([["New one", "Review"]]);
  getToastStore().undo(toasts[0]!.id);
  expect(reviewed).toEqual([1]);
});
