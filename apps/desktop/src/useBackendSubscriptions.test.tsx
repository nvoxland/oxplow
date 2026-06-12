import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { useRef, useState } from "react";
import { act, cleanup, render } from "@testing-library/react";

import { type BackendSubscriptionApi, useBackendSubscriptions } from "./useBackendSubscriptions.js";

// Captured oxplow-event handlers + unsubscribe counters, reset per test.
// The api surface is injected (not module-mocked) so nothing leaks into
// other test files sharing this bun process.
type Handler = (event: Record<string, unknown>) => void;
let oxplowHandlers: Handler[] = [];
let unsubCount = 0;
const getThreadWorkState = mock(async () => ({}));

function makeApi(): BackendSubscriptionApi {
  const noopSub = () => () => {
    unsubCount += 1;
  };
  return {
    subscribeWorkspaceContext: noopSub,
    subscribeBacklogEvents: noopSub,
    subscribeTaskEvents: noopSub,
    subscribeAgentStatus: noopSub,
    subscribeAgentStallAlerts: noopSub,
    subscribeOxplowEvents: ((handler: Handler) => {
      oxplowHandlers.push(handler);
      return () => {
        unsubCount += 1;
      };
    }) as never,
    getBacklogState: (async () => ({})) as never,
    getThreadState: (async () => ({})) as never,
    getThreadWorkState: getThreadWorkState as never,
    listStreams: (async () => []) as never,
    listAgentStatuses: (async () => []) as never,
    getConfig: (async () => ({ generated: [] })) as never,
  };
}

type ThreadStates = Record<string, { threads: { id: string }[] }>;

function makeHandlers(threadStatesRef: { current: ThreadStates }) {
  const noop = () => {};
  return {
    threadStatesRef: threadStatesRef as never,
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

function Harness({ threadStates }: { threadStates: ThreadStates }) {
  const ref = useRef(threadStates);
  ref.current = threadStates;
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
  unsubCount = 0;
  getThreadWorkState.mockClear();
});

afterEach(cleanup);

test("subscribes to the oxplow event bus on mount", () => {
  render(<Harness threadStates={{}} />);
  // followup.changed, thread.changed, stream prompt-changed,
  // streamsChanged, streamOrphaned, config.changed = 6 subscriptions.
  expect(oxplowHandlers.length).toBe(6);
});

test("does not re-subscribe across re-renders (no churn)", () => {
  const { getByText } = render(<Harness threadStates={{}} />);
  const afterMount = oxplowHandlers.length;
  act(() => {
    getByText("rerender").click();
  });
  expect(oxplowHandlers.length).toBe(afterMount);
});

test("unsubscribes every subscription on unmount", () => {
  const { unmount } = render(<Harness threadStates={{}} />);
  unmount();
  // 6 oxplow + workspace-context + backlog + task + agent-status +
  // stall-alerts = 11.
  expect(unsubCount).toBe(11);
});

test("followup.changed reads the current threadStates ref to recover the stream id", async () => {
  const threadStates: ThreadStates = { "s-1": { threads: [{ id: "t-1" }] } };
  render(<Harness threadStates={threadStates} />);

  await act(async () => {
    for (const handler of oxplowHandlers) {
      handler({ type: "followup.changed", threadId: "t-1" });
    }
    await Promise.resolve();
  });

  expect(getThreadWorkState).toHaveBeenCalledWith("s-1", "t-1");
});
