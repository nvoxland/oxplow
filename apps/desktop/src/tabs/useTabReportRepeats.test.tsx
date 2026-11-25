import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, renderHook } from "@testing-library/react";

// A socket close makes the daemon forget the window's tab report, so the
// window sends it again as soon as it reconnects, not only on the heartbeat.

const realApi = { ...(await import("../api.js")) }; // a copy: `mock.module` patches a loaded module in place
let reconnect: (() => void) | null = null;
mock.module("../api.js", () => ({
  ...realApi,
  onRemoteReconnect: (handler: () => void) => {
    reconnect = handler;
    return () => {
      reconnect = null;
    };
  },
}));
const { useTabReportRepeats } = await import("./useTabReportRepeats.js");

afterEach(() => cleanup());

const report = { focused: true, visible: true, shownThread: null, threads: [], detailJson: null, runs: ["r1"] } as never;

test("a reconnect sends the last report again, with the runs open now", () => {
  const sent: unknown[] = [];
  const last = { current: report };
  renderHook(() => useTabReportRepeats(last, { send: (r) => sent.push(r) }, () => ["r2"], 60_000));
  act(() => reconnect!());
  expect(sent).toEqual([{ ...(report as object), runs: ["r2"] }]);
  expect(last.current).toEqual(sent[0]);
});

test("nothing is sent before the window has reported", () => {
  const sent: unknown[] = [];
  renderHook(() => useTabReportRepeats({ current: null }, { send: (r) => sent.push(r) }, () => [], 60_000));
  act(() => reconnect!());
  expect(sent).toEqual([]);
});

test("the heartbeat sends it again too", async () => {
  const sent: unknown[] = [];
  renderHook(() => useTabReportRepeats({ current: report }, { send: (r) => sent.push(r) }, () => ["r1"], 10));
  await act(() => new Promise((resolve) => setTimeout(resolve, 35)));
  expect(sent.length).toBeGreaterThan(0);
});
