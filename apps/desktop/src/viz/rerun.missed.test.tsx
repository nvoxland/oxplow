import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, render } from "@testing-library/react";
import { useState } from "react";

// A change announced while a host's run was in flight, before the run said
// what it read, was dropped: the run may have read the old state, and
// nothing told the host to run again.

const realApi = await import("../api.js");
const listeners: Array<(e: Record<string, unknown>) => void> = [];
mock.module("../api.js", () => ({
  ...realApi,
  subscribeOxplowEvents: (l: (e: Record<string, unknown>) => void) => {
    listeners.push(l);
    return () => listeners.splice(listeners.indexOf(l), 1);
  },
}));
const { NO_READS, useRerunOnChange } = await import("./rerun.js");

afterEach(cleanup);

const emit = (e: Record<string, unknown>) => act(() => listeners.forEach((l) => l(e)));
const pause = (ms: number) => act(async () => void (await new Promise((r) => setTimeout(r, ms))));

test("a change announced before the run reported its reads re-runs the host once they match", async () => {
  let runs = 0;
  let report: (models: string[]) => void = () => {};
  function Host() {
    const [reads, setReads] = useState(NO_READS);
    report = (models) => setReads({ models, tables: [], measures: [] });
    useRerunOnChange(reads, () => {
      runs++;
    });
    return null;
  }
  render(<Host />);
  // The first run is in flight (its reads unknown): the write lands.
  emit({ kind: "modelsChanged", models: ["v_work_item"] });
  await pause(200);
  expect(runs).toBe(0);
  // It reports what it read, and that includes the model that changed.
  await act(async () => report(["v_work_item"]));
  await pause(200);
  expect(runs).toBe(1);
  // The re-run is the answer to that change: reporting again asks for no more.
  await act(async () => report(["v_work_item"]));
  await pause(200);
  expect(runs).toBe(1);
});

test("a change to a model the run didn't read asks for nothing", async () => {
  let runs = 0;
  let report: (models: string[]) => void = () => {};
  function Host() {
    const [reads, setReads] = useState(NO_READS);
    report = (models) => setReads({ models, tables: [], measures: [] });
    useRerunOnChange(reads, () => {
      runs++;
    });
    return null;
  }
  render(<Host />);
  emit({ kind: "modelsChanged", models: ["v_snapshot"] });
  await act(async () => report(["v_work_item"]));
  await pause(200);
  expect(runs).toBe(0);
});
