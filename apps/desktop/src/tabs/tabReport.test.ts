import { expect, test } from "bun:test";

import { buildTabReport, createTabReporter } from "./tabReport.js";

// A window reports only what is live; its tabs are the stored ones.
test("a window's report carries its focus, visibility, shown thread, detail and runs", () => {
  const report = buildTabReport({
    focused: true,
    visible: true,
    shownThread: "thr1",
    detail: { path: "src/a.rs" },
    runs: ["panel-1", "markdown-1", "panel-1"],
  });
  expect(report).toEqual({
    runs: ["markdown-1", "panel-1"],
    focused: true,
    visible: true,
    shown_thread: "thr1",
    detail_json: '{"path":"src/a.rs"}',
  });
});

test("a window with no page runs reports an explicit empty roster", () => {
  expect(buildTabReport({ focused: false, visible: true, shownThread: null, detail: null }).runs).toEqual([]);
});

const rosterReport = (runs: string[]) => buildTabReport({ runs, focused: true, visible: true, shownThread: null, detail: null });

test("report writes are serialized and only the newest pending roster is sent", async () => {
  const sent: string[][] = [];
  const release: (() => void)[] = [];
  const reporter = createTabReporter(async (report) => {
    sent.push(report.runs);
    await new Promise<void>((resolve) => release.push(resolve));
  });
  reporter.send(rosterReport(["old"]));
  reporter.send(rosterReport(["intermediate"]));
  reporter.send(rosterReport(["newest"]));
  expect(sent).toEqual([["old"]]);
  release[0]();
  await Promise.resolve();
  await Promise.resolve();
  expect(sent).toEqual([["old"], ["newest"]]);
  release[1]();
});

test("a failed report does not block a newer pending roster or a later heartbeat", async () => {
  const sent: string[][] = [];
  let reject!: (error: Error) => void;
  const reporter = createTabReporter(async (report) => {
    sent.push(report.runs);
    if (sent.length === 1) await new Promise<void>((_, fail) => { reject = fail; });
  });
  reporter.send(rosterReport(["old"]));
  reporter.send(rosterReport(["current"]));
  reject(new Error("temporary disconnect"));
  await Promise.resolve();
  await Promise.resolve();
  expect(sent).toEqual([["old"], ["current"]]);
  reporter.send(rosterReport(["heartbeat"]));
  await Promise.resolve();
  expect(sent.at(-1)).toEqual(["heartbeat"]);
});

test("the shown page's run and focused node reach the report as detail_json", () => {
  const report = buildTabReport({
    focused: true,
    visible: true,
    shownThread: null,
    detail: { runId: "run-1", focus: { node: "big" } },
  });
  expect(JSON.parse(report.detail_json!)).toEqual({ runId: "run-1", focus: { node: "big" } });
});
