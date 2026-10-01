import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, render, waitFor } from "@testing-library/react";

// A model read re-runs on `modelsChanged` for a model it read — through
// `useRerunOnChange` over the read's own `reads`, the one rerun mechanism
// every page uses — so a drifting ref (a new snapshot changes
// `v_knowledge_ref.stale`) shows without reopening the page.

type Handler = (event: Record<string, unknown>) => void;
const handlers: Handler[] = [];
let reads = 0;
const realApi = await import("../api.js");
// Bun's `mock.module` is process-wide: only this page's read is faked;
// everything else delegates to the real functions — captured before the
// mock, since the namespace's bindings become the mock afterwards.
const realQuerySql = realApi.querySql;
const realSubscribe = realApi.subscribeOxplowEvents;
mock.module("../api.js", () => ({
  ...realApi,
  subscribeOxplowEvents: (handler: Handler) => {
    handlers.push(handler);
    const off = realSubscribe(handler);
    return () => {
      handlers.splice(handlers.indexOf(handler), 1);
      off();
    };
  },
  querySql: async (sql: string, params: unknown[], limit?: number) => {
    if (!sql.includes("v_knowledge_ref")) return realQuerySql(sql, params as never, limit);
    reads += 1;
    return {
      columns: ["path", "pinned_snapshot_id", "pinned_vcs_rev", "pinned_vcs_rev_exact", "latest_snapshot_id", "stale"],
      rows: [["src/a.rs", 1, null, 0, reads, reads > 1 ? 1 : 0]],
      truncated: false,
      reads: { models: ["v_knowledge_ref"], tables: [], measures: [] },
      freshness: [],
    };
  },
}));
const { WikiFreshnessPage } = await import("./WikiFreshnessPage.js");

afterEach(cleanup);

const fire = async (event: Record<string, unknown>) => {
  await act(async () => {
    for (const h of [...handlers]) h(event);
    // `useRerunOnChange` coalesces a burst into one re-run.
    await new Promise((r) => setTimeout(r, 150));
  });
};

test("the freshness page re-reads when v_knowledge_ref changes, and only then", async () => {
  const { container } = render(<WikiFreshnessPage slug="notes" onOpenPage={() => {}} />);
  await waitFor(() => expect(reads).toBe(1));
  expect(container.textContent).toContain("src/a.rs");
  await fire({ kind: "modelsChanged", models: ["v_task"] });
  expect(reads).toBe(1);
  await fire({ kind: "modelsChanged", models: ["v_knowledge_ref"] });
  expect(reads).toBe(2);
});
