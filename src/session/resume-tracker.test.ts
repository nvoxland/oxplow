import { expect, test } from "bun:test";
import { decideResumeUpdate } from "./resume-tracker.js";

test("returns a set directive when a new session id is observed", () => {
  expect(decideResumeUpdate("", "session-1")).toEqual({ type: "set", sessionId: "session-1" });
});

test("returns a set directive when the session id changes (e.g. after compact)", () => {
  expect(decideResumeUpdate("old-session", "new-session")).toEqual({
    type: "set",
    sessionId: "new-session",
  });
});

test("returns null when the observed id matches what's already persisted", () => {
  expect(decideResumeUpdate("session-1", "session-1")).toBeNull();
});

test("returns null when no session id rode the hook payload", () => {
  expect(decideResumeUpdate("session-1", undefined)).toBeNull();
});
