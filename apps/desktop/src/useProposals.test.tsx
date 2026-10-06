import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, renderHook, waitFor } from "@testing-library/react";

// A pending-proposals read that fails says so (the op-error tray), rather
// than leaving the Alerts panel's proposal cards silently stale.

const realApi = await import("./api.js");
const realQuerySql = realApi.querySql;
let failing = false;
mock.module("./api.js", () => ({
  ...realApi,
  querySql: async (sql: string, ...rest: unknown[]) => {
    if (failing && sql.includes("v_command_proposal")) throw new Error("no such table: v_command_proposal");
    return (realQuerySql as (...a: unknown[]) => unknown)(sql, ...rest);
  },
}));
const { useProposals } = await import("./proposals.js");
const { getOpErrorsStore } = await import("./components/opErrorsStore.js");

afterEach(() => {
  failing = false;
  getOpErrorsStore().clear();
  cleanup();
});

test("a failed read of the pending proposals is reported", async () => {
  failing = true;
  renderHook(() => useProposals());
  await waitFor(() =>
    expect(getOpErrorsStore().getSnapshot().map((e) => e.message)).toContain("no such table: v_command_proposal"),
  );
});
