import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { useRef, useState } from "react";
import { act, cleanup, render } from "@testing-library/react";

import { type BackendSubscriptionApi, useBackendSubscriptions } from "./useBackendSubscriptions.js";

// Captured oxplow-event handlers + unsubscribe counters, reset per test.
// The api surface is injected (not module-mocked) so nothing leaks into
// other test files sharing this bun process.
type Handler = (event: Record<string, unknown>) => void;
let oxplowHandlers: Handler[] = [];
let reconnectHandlers: Array<() => void> = [];
let unsubCount = 0;
// What the reads read: the hook re-runs them on a change to those models
// (`readsChanged`), not on a hard-coded list.
const taskReads = { reads: { models: ["v_task", "v_task_note"], tables: [], measures: [] } };
const readThreadWork = mock(async () => taskReads);
const readBacklog = mock(async () => taskReads);
const listAgentStatuses = mock(async () => []);
const getConfig = mock(async () => ({ generated: { exclude: [], include: [] } }));
const getThreadState = mock(async () => ({ selectedThreadId: null, activeThreadId: null, threads: [] }));

function makeApi(): BackendSubscriptionApi {
  const noopSub = () => () => {
    unsubCount += 1;
  };
  return {
    subscribeWorkspaceContext: noopSub,
    subscribeAgentStatus: noopSub,
    subscribeAgentStallAlerts: noopSub,
    subscribeOxplowEvents: ((handler: Handler) => {
      oxplowHandlers.push(handler);
      return () => {
        unsubCount += 1;
      };
    }) as never,
    onRemoteReconnect: ((handler: () => void) => {
      reconnectHandlers.push(handler);
      return () => {
        unsubCount += 1;
      };
    }) as never,
    readBacklog: readBacklog as never,
    getThreadState: getThreadState as never,
    readThreadWork: readThreadWork as never,
    listStreams: (async () => []) as never,
    listAgentStatuses: listAgentStatuses as never,
    getConfig: getConfig as never,
  };
}

type WorkStates = Record<string, unknown>;

function makeHandlers(threadWorkStatesRef: { current: WorkStates }) {
  const noop = () => {};
  return {
    threadWorkStatesRef: threadWorkStatesRef as never,
    // Two streams' thread state is loaded.
    threadStatesRef: { current: { str1: {}, str2: {} } } as never,
    setWorkspaceContext: noop,
    setBacklogState: noop,
    setThreadWorkStates: mock(noop) as never,
    setThreadStates: noop as never,
    setStreams: noop as never,
    setStream: noop as never,
    setAgentStatuses: noop as never,
    setGeneratedState: noop,
    setEnabledAgents: noop,
  };
}

function Harness({ workStates }: { workStates: WorkStates }) {
  const ref = useRef(workStates);
  ref.current = workStates;
  const [, bump] = useState(0);
  // Build handlers + api once — in the real App these are stable, so the
  // subscriptions must not churn across renders.
  const stable = useRef<{ handlers: ReturnType<typeof makeHandlers>; api: BackendSubscriptionApi } | null>(null);
  if (!stable.current) stable.current = { handlers: makeHandlers(ref), api: makeApi() };
  useBackendSubscriptions(stable.current.handlers as never, stable.current.api);
  return <button onClick={() => bump((n) => n + 1)}>rerender</button>;
}

beforeEach(() => {
  oxplowHandlers = [];
  reconnectHandlers = [];
  unsubCount = 0;
  readThreadWork.mockClear();
  readBacklog.mockClear();
  listAgentStatuses.mockClear();
  getConfig.mockClear();
});

afterEach(cleanup);

test("subscribes to the oxplow event bus on mount", () => {
  render(<Harness workStates={{}} />);
  // tasks (models + followups), threads (v_thread), streamsChanged,
  // streamOrphaned, configChanged = 5 subscriptions.
  expect(oxplowHandlers.length).toBe(5);
});

test("does not re-subscribe across re-renders (no churn)", () => {
  const { getByText } = render(<Harness workStates={{}} />);
  const afterMount = oxplowHandlers.length;
  act(() => {
    getByText("rerender").click();
  });
  expect(oxplowHandlers.length).toBe(afterMount);
});

test("unsubscribes every subscription on unmount", () => {
  const { unmount } = render(<Harness workStates={{}} />);
  unmount();
  // 5 oxplow + workspace-context + agent-status + stall-alerts = 8, plus
  // 3 reconnect handlers (tasks, config, agent-status) = 11.
  expect(unsubCount).toBe(11);
});

test("registers reconnect handlers for the core stores", () => {
  render(<Harness workStates={{}} />);
  // tasks, config, agent-status re-hydrate on a remote WS reconnect.
  expect(reconnectHandlers.length).toBe(3);
});

test("re-hydrates core stores on a remote reconnect", async () => {
  render(<Harness workStates={{}} />);
  // One fetch each on mount.
  expect(readBacklog).toHaveBeenCalledTimes(1);
  expect(getConfig).toHaveBeenCalledTimes(1);
  expect(listAgentStatuses).toHaveBeenCalledTimes(1);

  await act(async () => {
    for (const handler of reconnectHandlers) handler();
    await Promise.resolve();
  });

  // A second fetch each after the reconnect fired.
  expect(readBacklog).toHaveBeenCalledTimes(2);
  expect(getConfig).toHaveBeenCalledTimes(2);
  expect(listAgentStatuses).toHaveBeenCalledTimes(2);
});

test("a change to a model the reads read re-reads the backlog and every loaded thread; followups re-read their thread", async () => {
  render(<Harness workStates={{ thr1: taskReads, thr2: taskReads }} />);
  // Let the mount's reads land, so the hook knows what they read.
  await act(async () => {
    await Promise.resolve();
  });
  readThreadWork.mockClear();
  readBacklog.mockClear();
  await act(async () => {
    for (const handler of oxplowHandlers) handler({ kind: "modelsChanged", models: ["v_task_note"] });
    await Promise.resolve();
  });
  expect(readBacklog).toHaveBeenCalledTimes(1);
  expect(readThreadWork.mock.calls.map((c) => (c as unknown[])[0])).toEqual(["thr1", "thr2"]);

  readThreadWork.mockClear();
  readBacklog.mockClear();
  await act(async () => {
    // A model nothing here read — and a model a task read *could* name
    // but these didn't (what the hook knows comes from the reads, not a
    // list of task models).
    for (const handler of oxplowHandlers) handler({ kind: "modelsChanged", models: ["v_commit"] });
    for (const handler of oxplowHandlers) handler({ kind: "modelsChanged", models: ["v_effort_file"] });
    for (const handler of oxplowHandlers) handler({ kind: "followupsChanged", threadId: "thr3" });
    await Promise.resolve();
  });
  expect(readBacklog).toHaveBeenCalledTimes(0);
  expect(readThreadWork.mock.calls.map((c) => (c as unknown[])[0])).toEqual(["thr3"]);
});

test("a change to v_thread re-reads every loaded stream's thread state, and nothing else does", async () => {
  render(<Harness workStates={{}} />);
  getThreadState.mockClear();
  await act(async () => {
    for (const handler of oxplowHandlers) handler({ kind: "modelsChanged", models: ["v_commit"] });
    await Promise.resolve();
  });
  expect(getThreadState).toHaveBeenCalledTimes(0);
  await act(async () => {
    for (const handler of oxplowHandlers) handler({ kind: "modelsChanged", models: ["v_thread"] });
    await Promise.resolve();
  });
  expect(getThreadState.mock.calls.map((c) => (c as unknown[])[0])).toEqual(["str1", "str2"]);
});
