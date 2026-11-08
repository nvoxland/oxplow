import { describe, expect, test } from "bun:test";

import { opErrorsFromResult, reportOpErrorWith } from "./opErrors.js";

describe("reportOpErrorWith", () => {
  test("reports an op error to the daemon as ui.report_error, fire-and-forget", async () => {
    const calls: Array<{ name: string; input: unknown }> = [];
    const run = async (name: string, input: unknown) => {
      calls.push({ name, input });
      return {};
    };
    const at = { thread: "thr3", stream: "str1" };
    reportOpErrorWith(run, () => {}, {
      label: "List data",
      command: "query_sql",
      message: "IpcCallError: query_sql: timed out after 5s",
      stderr: "timed out",
      exitCode: 1,
      durationMs: 5012.4,
      signal: "SIGTERM",
    }, at);
    reportOpErrorWith(run, () => {}, { label: "Save note", threadId: null }, at);
    reportOpErrorWith(run, () => {}, { label: "Approve", auditId: 41, proposalId: 3, threadId: null }, { thread: null, stream: null });
    await Promise.resolve();
    expect(calls).toEqual([
      {
        name: "oxplow.ui.report_error",
        input: {
          label: "List data",
          command: "query_sql",
          message: "IpcCallError: query_sql: timed out after 5s",
          stderr: "timed out",
          exit_code: 1,
          thread: "thr3",
          signal: "SIGTERM",
          duration_ms: 5012,
        },
      },
      // From no thread, it names the stream on screen, so that stream's
      // agent can read its output.
      { name: "oxplow.ui.report_error", input: { label: "Save note", stream: "str1" } },
      // What recorded the run: its audit row, and an approval's proposal.
      { name: "oxplow.ui.report_error", input: { label: "Approve", audit_id: 41, proposal: 3 } },
    ]);
  });

  test("a failed report is logged, never recorded as another op error", async () => {
    const logged: Array<{ level: string; message: string }> = [];
    const calls: string[] = [];
    const run = async (name: string) => {
      calls.push(name);
      throw new Error("daemon unreachable");
    };
    reportOpErrorWith(run, (level, message) => logged.push({ level, message }), { label: "Push" }, { thread: null, stream: null });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).toEqual(["oxplow.ui.report_error"]);
    expect(logged).toEqual([{ level: "warn", message: "couldn't report an op error to the daemon" }]);
  });

  test("a report resolves to the failure's id, the row's event id; a failed report to null", async () => {
    const at = { thread: null, stream: null };
    const ok = async () => ({ result: { event: "01ev" }, audit_id: 1, event_id: "x", inverse: null });
    expect(await reportOpErrorWith(ok, () => {}, { label: "Push" }, at)).toBe("01ev");
    const broken = async () => {
      throw new Error("down");
    };
    expect(await reportOpErrorWith(broken, () => {}, { label: "Push" }, at)).toBeNull();
  });
});

test("v_op_error rows are failed operations, with whether the person has seen each", () => {
  const errors = opErrorsFromResult({
    columns: ["event_id", "at", "label", "command", "message", "exit_code", "signal", "duration_ms", "output_size", "seen"],
    rows: [
      ["e1", "2026-10-22T10:00:00Z", "Push", "git push", null, 1, null, 40, 12, 1],
      ["e2", "2026-10-22T09:00:00Z", "Save", null, "disk full", null, "SIGTERM", null, null, 0],
    ],
    truncated: false,
    reads: { datasets: ["v_op_error"], tables: [], measures: [] },
    freshness: {},
  } as never);
  expect(errors).toEqual([
    { id: "e1", at: Date.parse("2026-10-22T10:00:00Z"), label: "Push", command: "git push", message: null, exitCode: 1, signal: null, durationMs: 40, outputSize: 12, seen: true },
    { id: "e2", at: Date.parse("2026-10-22T09:00:00Z"), label: "Save", command: null, message: "disk full", exitCode: null, signal: "SIGTERM", durationMs: null, outputSize: null, seen: false },
  ]);
});
