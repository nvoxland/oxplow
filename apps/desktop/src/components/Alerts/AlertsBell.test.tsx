import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, within, fireEvent, render, waitFor } from "@testing-library/react";

// Everything that needs the person is counted by the status bar's bell and
// announced by a toast once; the Alerts page itself is a foundation lens.

const realApi = await import("../../api.js");
/** Any other read (the Go To section's wiki pages, …): no rows. Never the
 *  real call — with no Tauri host it throws, and the test would pass only
 *  where another file's mock answered first (tsk1012). */
const noRows = { columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} };
let extensionRows: unknown[][] = [];
/** The failed operations `v_op_error` answers with. */
let opErrorRows: unknown[][] = [];
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
        reads: { models: ["v_extension"], tables: [], measures: [] },
        freshness: {},
      };
    }
    if (sql.includes("FROM v_op_error")) {
      return {
        columns: ["event_id", "at", "label", "command", "message", "exit_code", "signal", "duration_ms", "output_size", "seen"],
        rows: opErrorRows,
        truncated: false,
        reads: { models: ["v_op_error"], tables: [], measures: [] },
        freshness: {},
      };
    }
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
        columns: ["id", "uri", "created_at", "command", "input", "actor_kind", "actor_id", "thread_id", "key", "preview", "dry_run"],
        rows: [[7, "oxplow:oxplow/proposal?id=7", "t", "oxplow.config.set", '{"key":"zones","value":[]}', "agent", "thr3", 3, "oxplow:oxplow/config?key=zones", '{"command":"oxplow.config.set","summary":"Set zones","input":{},"destructive":false}', null]],
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
const { AlertsIndicator } = await import("./AlertsIndicator.js");
const { PanelRunsProvider } = await import("../Panels/PanelRunsContext.js");
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
  opErrorRows = [];
  cleanup();
});

test("the bell counts everything, red for a problem, and opens Alerts", async () => {
  opErrorRows = [pushFailed];
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
