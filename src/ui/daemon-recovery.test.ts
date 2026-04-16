import { expect, test } from "bun:test";
import { advanceDaemonProbeState, INITIAL_DAEMON_PROBE_STATE } from "./daemon-recovery.js";

test("healthy probe keeps the daemon marked available", () => {
  expect(advanceDaemonProbeState(INITIAL_DAEMON_PROBE_STATE, true)).toEqual({
    next: INITIAL_DAEMON_PROBE_STATE,
    refresh: false,
  });
});

test("single failed probe does not mark the daemon unavailable yet", () => {
  expect(advanceDaemonProbeState(INITIAL_DAEMON_PROBE_STATE, false)).toEqual({
    next: {
      consecutiveFailures: 1,
      unavailable: false,
    },
    refresh: false,
  });
});

test("daemon becomes unavailable only after repeated failed probes", () => {
  const once = advanceDaemonProbeState(INITIAL_DAEMON_PROBE_STATE, false).next;
  const twice = advanceDaemonProbeState(once, false).next;
  expect(advanceDaemonProbeState(twice, false)).toEqual({
    next: {
      consecutiveFailures: 3,
      unavailable: true,
    },
    refresh: false,
  });
});

test("recovery refresh only happens after the daemon was marked unavailable", () => {
  const downState = { consecutiveFailures: 3, unavailable: true };
  expect(advanceDaemonProbeState(downState, true)).toEqual({
    next: INITIAL_DAEMON_PROBE_STATE,
    refresh: true,
  });
});
