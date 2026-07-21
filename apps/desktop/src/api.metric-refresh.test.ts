import { afterEach, beforeEach, expect, mock, test } from "bun:test";

// tsk197: metric consumers (MetricTile, MetricDetailPage) refresh on every
// `metricSamplesChanged`, which the OTLP token ingest emits every ~10s while an
// agent runs. `subscribeMetricRefresh` coalesces that burst (trailing debounce)
// while keeping `configChanged` — a user action — immediate.

// Mock the true seam — transport's `listen` — rather than `subscribeOxplowEvents`:
// the latter is called intra-module by `subscribeMetricRefresh`, so an ESM export
// mock would not intercept it. Events are pushed through `emit`.
let handlers: Array<(e: { payload: { kind: string; measures?: string[] } }) => void> = [];
const emit = (kind: string, measures: string[] = []) =>
  handlers.forEach((h) => h({ payload: { kind, measures } }));

const realTransport = await import("./tauri-bridge/transport.js");
mock.module("./tauri-bridge/transport.js", () => ({
  ...realTransport,
  listen: async (
    _channel: string,
    handler: (e: { payload: { kind: string; measures?: string[] } }) => void,
  ) => {
    handlers.push(handler);
    return () => {
      handlers = handlers.filter((h) => h !== handler);
    };
  },
}));

const { metricRefreshAction, subscribeMetricRefresh } = await import("./api.js");

beforeEach(() => {
  handlers = [];
});
afterEach(() => {});

const ev = (kind: string, measures: string[] = []) => ({ kind, measures });

test("metricRefreshAction routes each event kind", () => {
  // metricSamplesChanged is the OTLP burst — always debounced.
  expect(metricRefreshAction(ev("metricSamplesChanged"), {})).toBe("debounce");
  expect(metricRefreshAction(ev("metricSamplesChanged"), { alsoConfig: true })).toBe("debounce");
  // configChanged is a user action — immediate, but only when opted in.
  expect(metricRefreshAction(ev("configChanged"), { alsoConfig: true })).toBe("now");
  expect(metricRefreshAction(ev("configChanged"), {})).toBe("ignore");
  // everything else is irrelevant to a metric view.
  expect(metricRefreshAction(ev("workspaceChanged"), { alsoConfig: true })).toBe("ignore");
});

test("metricRefreshAction filters by measure scope, failing open on either side", () => {
  const opts = { measures: ["oxplow.tokens"] };
  // Disjoint: a token export can't affect a coverage tile — skip it.
  expect(metricRefreshAction(ev("metricSamplesChanged", ["oxplow.coverage"]), opts)).toBe("ignore");
  // Overlapping: the tile's own measure changed — debounce.
  expect(metricRefreshAction(ev("metricSamplesChanged", ["oxplow.tokens"]), opts)).toBe("debounce");
  // Event names no measures (an un-migrated emit site) — fail open, refresh.
  expect(metricRefreshAction(ev("metricSamplesChanged", []), opts)).toBe("debounce");
  // Consumer names no measures (a formula metric / whole-page view) — fail open.
  expect(metricRefreshAction(ev("metricSamplesChanged", ["oxplow.coverage"]), {})).toBe("debounce");
  // The measure filter never suppresses a configChanged refresh.
  expect(
    metricRefreshAction(ev("configChanged", ["oxplow.coverage"]), { alsoConfig: true, ...opts }),
  ).toBe("now");
});

test("a burst of metricSamplesChanged collapses to one trailing refresh", async () => {
  let calls = 0;
  const off = subscribeMetricRefresh(() => calls++, { debounceMs: 50 });

  emit("metricSamplesChanged");
  emit("metricSamplesChanged");
  emit("metricSamplesChanged");
  expect(calls).toBe(0); // trailing — nothing yet

  await new Promise((r) => setTimeout(r, 80));
  expect(calls).toBe(1); // the whole burst became one reload
  off();
});

test("configChanged refreshes immediately when alsoConfig is set", async () => {
  let calls = 0;
  const off = subscribeMetricRefresh(() => calls++, { debounceMs: 50, alsoConfig: true });

  emit("configChanged");
  expect(calls).toBe(1); // immediate, no wait
  off();
});

test("configChanged is ignored when alsoConfig is not set", async () => {
  let calls = 0;
  const off = subscribeMetricRefresh(() => calls++, { debounceMs: 50 });

  emit("configChanged");
  await new Promise((r) => setTimeout(r, 80));
  expect(calls).toBe(0);
  off();
});

test("a scoped consumer ignores a disjoint measure event but not its own", async () => {
  let calls = 0;
  const off = subscribeMetricRefresh(() => calls++, {
    debounceMs: 50,
    measures: ["oxplow.tokens"],
  });

  emit("metricSamplesChanged", ["oxplow.coverage"]); // not mine — skip
  await new Promise((r) => setTimeout(r, 80));
  expect(calls).toBe(0);

  emit("metricSamplesChanged", ["oxplow.tokens"]); // mine — reload
  await new Promise((r) => setTimeout(r, 80));
  expect(calls).toBe(1);
  off();
});

test("unsubscribing cancels a pending debounced refresh", async () => {
  let calls = 0;
  const off = subscribeMetricRefresh(() => calls++, { debounceMs: 50 });

  emit("metricSamplesChanged");
  off(); // tears down before the timer fires

  await new Promise((r) => setTimeout(r, 80));
  expect(calls).toBe(0); // the pending reload was cancelled
});
