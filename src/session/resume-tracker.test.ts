import { expect, test } from "bun:test";
import { ResumeTracker } from "./resume-tracker.js";

test("session start persists the pane resume id", () => {
  const tracker = new ResumeTracker();

  tracker.notePaneLaunch("stream-1", "working", true);
  const update = tracker.recordHookEvent("stream-1", "working", "SessionStart", "session-1");

  expect(update).toEqual({ type: "set", sessionId: "session-1" });
});

test("session end without a prior session start clears a failed resumed pane", () => {
  const tracker = new ResumeTracker();

  tracker.notePaneLaunch("stream-1", "working", true);
  const update = tracker.recordHookEvent("stream-1", "working", "SessionEnd", "session-end-1");

  expect(update).toEqual({ type: "clear" });
});

test("session end after a successful session start keeps the resume id unchanged", () => {
  const tracker = new ResumeTracker();

  tracker.notePaneLaunch("stream-1", "working", true);
  tracker.recordHookEvent("stream-1", "working", "SessionStart", "session-1");
  const update = tracker.recordHookEvent("stream-1", "working", "SessionEnd", "session-1");

  expect(update).toBeNull();
});
