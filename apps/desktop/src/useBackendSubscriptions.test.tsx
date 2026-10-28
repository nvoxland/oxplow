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
const listAgentStatuses = mock(async (): Promise<unknown[]> => []);
const setSessionStatuses = mock((_next: unknown) => {});
const getThreadState = mock(async () => ({ selectedThreadId: null, threads: [] }));

function makeApi(): BackendSubscriptionApi {
  const noopSub = () => () => {
    unsubCount += 1;
  };
  return {
    subscribeWorkspaceContext: noopSub,
    subscribeAgentStatus: noopSub,
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
    getThreadState: getThreadState as never,
    listStreams: (async () => []) as never,
    listAgentStatuses: listAgentStatuses as never,
  };
}

function makeHandlers() {
  const noop = () => {};
  return {
    // Two streams' thread state is loaded.
    threadStatesRef: { current: { str1: {}, str2: {} } } as never,
    setWorkspaceContext: noop,
    setThreadStates: noop as never,
    setStreams: noop as never,
    setStream: noop as never,
    setSessionStatuses: setSessionStatuses as never,
    setEnabledAgents: noop,
  };
}

function Harness() {
  const [, bump] = useState(0);
  // Build handlers + api once — in the real App these are stable, so the
  // subscriptions must not churn across renders.
  const stable = useRef<{ handlers: ReturnType<typeof makeHandlers>; api: BackendSubscriptionApi } | null>(null);
  if (!stable.current) stable.current = { handlers: makeHandlers(), api: makeApi() };
  useBackendSubscriptions(stable.current.handlers as never, stable.current.api);
  return <button onClick={() => bump((n) => n + 1)}>rerender</button>;
}

beforeEach(() => {
  oxplowHandlers = [];
  reconnectHandlers = [];
  unsubCount = 0;
  listAgentStatuses.mockClear();
  setSessionStatuses.mockClear();
});

afterEach(cleanup);

test("subscribes to the oxplow event bus on mount", () => {
  render(<Harness />);
  // threads (v_thread), streams (v_stream), streamOrphaned,
  // agent sessions (v_agent_session) = 4 subscriptions.
  expect(oxplowHandlers.length).toBe(4);
});

test("does not re-subscribe across re-renders (no churn)", () => {
  const { getByText } = render(<Harness />);
  const afterMount = oxplowHandlers.length;
  act(() => {
    getByText("rerender").click();
  });
  expect(oxplowHandlers.length).toBe(afterMount);
});

test("unsubscribes every subscription on unmount", () => {
  const { unmount } = render(<Harness />);
  unmount();
  // 4 oxplow + workspace-context + agent-status = 6, plus the reconnect
  // handler (agent-status) = 7.
  expect(unsubCount).toBe(7);
});

test("registers reconnect handlers for the core stores", () => {
  render(<Harness />);
  // agent-status re-hydrates on a remote WS reconnect.
  expect(reconnectHandlers.length).toBe(1);
});

test("two sessions in one thread keep their own statuses", async () => {
  const entry = (sessionId: string | null, status: string) => ({ streamId: "", threadId: "thr1", sessionId, status });
  listAgentStatuses.mockImplementationOnce(async () => [entry("ses1", "working"), entry("ses2", "awaiting"), entry(null, "waiting")]);
  render(<Harness />);
  await act(async () => {
    await Promise.resolve();
  });
  expect(setSessionStatuses).toHaveBeenCalledWith({
    ses1: entry("ses1", "working"),
    ses2: entry("ses2", "awaiting"),
    "thr1:unclaimed": entry(null, "waiting"),
  });
});

test("a change to the agent sessions re-reads the statuses: a closed one's drops out", async () => {
  render(<Harness />);
  expect(listAgentStatuses).toHaveBeenCalledTimes(1);
  await act(async () => {
    for (const handler of oxplowHandlers) handler({ kind: "datasetsChanged", datasets: ["v_agent_session"] });
    await Promise.resolve();
  });
  expect(listAgentStatuses).toHaveBeenCalledTimes(2);
});

test("re-hydrates core stores on a remote reconnect", async () => {
  render(<Harness />);
  // One fetch on mount.
  expect(listAgentStatuses).toHaveBeenCalledTimes(1);

  await act(async () => {
    for (const handler of reconnectHandlers) handler();
    await Promise.resolve();
  });

  // A second fetch after the reconnect fired.
  expect(listAgentStatuses).toHaveBeenCalledTimes(2);
});

test("a change to v_thread re-reads every loaded stream's thread state, and nothing else does", async () => {
  render(<Harness />);
  getThreadState.mockClear();
  await act(async () => {
    for (const handler of oxplowHandlers) handler({ kind: "datasetsChanged", datasets: ["v_commit"] });
    await Promise.resolve();
  });
  expect(getThreadState).toHaveBeenCalledTimes(0);
  await act(async () => {
    for (const handler of oxplowHandlers) handler({ kind: "datasetsChanged", datasets: ["v_thread"] });
    await Promise.resolve();
  });
  expect(getThreadState.mock.calls.map((c) => (c as unknown[])[0])).toEqual(["str1", "str2"]);
});
