import { expect, test } from "bun:test";
import { advanceDaemonProbeState, INITIAL_DAEMON_PROBE_STATE } from "./daemon-recovery.js";

test("healthy probe keeps the daemon marked available", () => {
  expect(advanceDaemonProbeState(INITIAL_DAEMON_PROBE_STATE, true)).toEqual({
    next: INITIAL_DAEMON_PROBE_STATE,
    refresh: false,
  });
});

test("single failed probe marks the daemon unavailable immediately", () => {
  expect(advanceDaemonProbeState(INITIAL_DAEMON_PROBE_STATE, false)).toEqual({
    next: {
      unavailable: true,
    },
    refresh: false,
  });
});

test("additional failed probes keep the daemon unavailable without extra transitions", () => {
  const downState = advanceDaemonProbeState(INITIAL_DAEMON_PROBE_STATE, false).next;
  expect(advanceDaemonProbeState(downState, false)).toEqual({
    next: {
      unavailable: true,
    },
    refresh: false,
  });
});

test("recovery refresh only happens after the daemon was marked unavailable", () => {
  const downState = { unavailable: true };
  expect(advanceDaemonProbeState(downState, true)).toEqual({
    next: INITIAL_DAEMON_PROBE_STATE,
    refresh: true,
  });
});
