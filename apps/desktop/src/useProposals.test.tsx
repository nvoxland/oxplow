import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, renderHook, waitFor } from "@testing-library/react";

// A pending-proposals read that fails says so (it is reported as a failed
// operation), rather than leaving the Alerts panel's proposal cards
// silently stale.

const realApi = await import("./api.js");
const realQuerySql = realApi.querySql;
let failing = false;
const reported: unknown[] = [];
mock.module("./api.js", () => ({
  ...realApi,
  querySql: async (sql: string, ...rest: unknown[]) => {
    if (failing && sql.includes("v_command_proposal")) throw new Error("no such table: v_command_proposal");
    return (realQuerySql as (...a: unknown[]) => unknown)(sql, ...rest);
  },
  runCommand: async (name: string, input: unknown) => {
    if (name === "oxplow.ui.report_error") reported.push(input);
    return { result: null, audit_id: 1, event_id: "e", inverse: null };
  },
}));
const { useProposals } = await import("./proposals.js");

afterEach(() => {
  failing = false;
  reported.length = 0;
  cleanup();
});

test("a failed read of the pending proposals is reported", async () => {
  failing = true;
  renderHook(() => useProposals());
  await waitFor(() =>
    expect(reported.map((r) => (r as { message?: string }).message)).toContain("no such table: v_command_proposal"),
  );
});
