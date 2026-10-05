import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// P6b.A4: Alerts says when proposals wait; its row brings the Approvals
// panel back even when the person had hidden it.

const realApi = await import("../../api.js");
/** Any other read (the Go To section's wiki pages, …): no rows. Never the
 *  real call — with no Tauri host it throws, and the test would pass only
 *  where another file's mock answered first (tsk1012). */
const noRows = { columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} };
const saved: unknown[] = [];
mock.module("../../api.js", () => ({
  ...realApi,
  getPanelLayout: async () => [{ panel: "core:approvals", hidden: true, collapsed: false }],
  setPanelLayout: async (layout: unknown) => {
    saved.push(layout);
  },
  listExtensions: async () => [],
  listCommentsForStream: async () => [],
  listRecentPageVisits: async () => [],
  topVisitedPages: async () => [],
  subscribeCommentEvents: () => () => {},
  subscribePageVisitEvents: () => () => {},
  subscribeOxplowEvents: () => () => {},
  querySql: async (sql: string) => {
    if (sql.includes("FROM v_command_proposal")) {
      return {
        columns: ["id", "ref", "created_at", "command", "input", "actor_kind", "actor_id", "thread_id", "key", "preview", "dry_run"],
        rows: [[7, "proposal:7", "t", "config.set", '{"key":"zones","value":[]}', "agent", "thr3", 3, "config:zones", '{"command":"config.set","summary":"Set zones","input":{},"destructive":false}', null]],
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
          [9, "plugin.repair", 42, "plugin.disabled", "boom", 1, "t"],
        ],
        truncated: false,
        reads: { models: ["v_event_dead_letter"], tables: [], measures: [] },
        freshness: {},
      };
    }
    return noRows;
  },
}));
const { RailHud } = await import("./RailHud.js");
const { indexRef } = await import("../../tabs/pageRefs.js");

afterEach(() => {
  saved.length = 0;
  cleanup();
});

test("the Alerts row for waiting proposals reveals a hidden Approvals panel", async () => {
  const view = render(<RailHud threadId={null} streamId={null} threadWork={null} onOpenPage={() => {}} />);
  const row = await waitFor(() => view.getByTestId("rail-alert-proposals"));
  expect(row.textContent).toContain("1 proposal awaits your approval");
  expect(view.queryByTestId("rail-section-core:approvals")).toBeNull();
  fireEvent.click(row);
  await waitFor(() => expect(view.getByTestId("rail-section-core:approvals")).toBeTruthy());
  expect(view.getByTestId("proposal-7")).toBeTruthy();
  const last = saved.at(-1) as Array<{ panel: string; hidden: boolean; collapsed: boolean }>;
  expect(last.find((p) => p.panel === "core:approvals")).toMatchObject({ hidden: false, collapsed: false });
});

// P7.C3: one Alerts row while events couldn't be delivered; it opens
// Settings, where Data → Delivery lists them.
test("the Alerts row for undelivered events opens Settings", async () => {
  const opened: unknown[] = [];
  const view = render(<RailHud threadId={null} streamId={null} threadWork={null} onOpenPage={(r) => opened.push(r)} />);
  const row = await waitFor(() => view.getByTestId("rail-alert-delivery"));
  expect(row.textContent).toBe("2 events couldn't be delivered");
  fireEvent.click(row);
  expect(opened).toEqual([indexRef("settings")]);
});
