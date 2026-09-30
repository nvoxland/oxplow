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
const readThreadWork = mock(async () => ({}));
const readBacklog = mock(async () => ({}));
const listAgentStatuses = mock(async () => []);
const getConfig = mock(async () => ({ generated: { exclude: [], include: [] } }));

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
    getThreadState: (async () => ({})) as never,
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
  // tasks (models + followups), threadsChanged, streamsChanged,
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

test("a task model change re-reads the backlog and every loaded thread; followups re-read their thread", async () => {
  render(<Harness workStates={{ thr1: {}, thr2: {} }} />);
  readThreadWork.mockClear();
  readBacklog.mockClear();
  await act(async () => {
    for (const handler of oxplowHandlers) handler({ kind: "modelsChanged", models: ["v_task"] });
    await Promise.resolve();
  });
  expect(readBacklog).toHaveBeenCalledTimes(1);
  expect(readThreadWork.mock.calls.map((c) => (c as unknown[])[0])).toEqual(["thr1", "thr2"]);

  readThreadWork.mockClear();
  await act(async () => {
    for (const handler of oxplowHandlers) handler({ kind: "modelsChanged", models: ["v_commit"] });
    for (const handler of oxplowHandlers) handler({ kind: "followupsChanged", threadId: "thr3" });
    await Promise.resolve();
  });
  expect(readThreadWork.mock.calls.map((c) => (c as unknown[])[0])).toEqual(["thr3"]);
});
