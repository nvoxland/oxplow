import { expect, test } from "bun:test";
import { shouldRefreshAfterDaemonRecovery } from "./daemon-recovery.js";

test("does not refresh on initial healthy state", () => {
  expect(shouldRefreshAfterDaemonRecovery(false, true)).toBe(false);
});

test("does not refresh while daemon remains unavailable", () => {
  expect(shouldRefreshAfterDaemonRecovery(true, false)).toBe(false);
});

test("refreshes when daemon recovers after being unavailable", () => {
  expect(shouldRefreshAfterDaemonRecovery(true, true)).toBe(true);
});
