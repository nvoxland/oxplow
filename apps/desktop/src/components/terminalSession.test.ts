import { expect, test } from "bun:test";

import { exitBanner, failedLaunch, isSessionGone, isStopped, mayStartNow, readSessionMessage, reopened } from "./terminalSession.js";

// tsk1026: after the daemon restarted, the agent pane kept the dead session's
// screen and swallowed keystrokes; an agent that exited said nothing.

test("an exit message is read, with its code", () => {
  expect(readSessionMessage(JSON.stringify({ type: "exit", exitCode: 0 }))).toEqual({
    kind: "exit",
    exitCode: 0,
    outcome: { kind: "ended" },
  });
  expect(readSessionMessage(JSON.stringify({ type: "exit" }))).toEqual({
    kind: "exit",
    exitCode: null,
    outcome: { kind: "ended" },
  });
});

test("a data message is read as bytes; anything else is nothing", () => {
  const read = readSessionMessage(JSON.stringify({ type: "data", bytes: btoa("hi") }));
  expect(read?.kind).toBe("data");
  expect(read && read.kind === "data" ? new TextDecoder().decode(read.bytes) : null).toBe("hi");
  expect(readSessionMessage("not json")).toBeNull();
  expect(readSessionMessage(JSON.stringify({ type: "resize" }))).toBeNull();
});

test("a send to a session the daemon doesn't know means it is gone", () => {
  expect(isSessionGone(new Error("IpcCallError: terminal session not found: term-1"))).toBe(true);
  expect(isSessionGone("terminal session not found: term-1")).toBe(true);
  expect(isSessionGone(new Error("the terminal didn't answer within 5 s"))).toBe(false);
});

test("opening again keeps a session that survived and replaces one that didn't", () => {
  expect(reopened("term-1", "term-1")).toBe("same");
  expect(reopened("term-1", "term-2")).toBe("new");
  expect(reopened(null, "term-2")).toBe("new");
});

test("a pane that starts on activation waits for its tab to be shown", () => {
  expect(mayStartNow(true, false)).toBe(false);
  expect(mayStartNow(true, true)).toBe(true);
});

test("any other pane opens at once", () => {
  expect(mayStartNow(false, false)).toBe(true);
});

test("a session the person stopped is never started by showing it", () => {
  expect(mayStartNow(true, true, true)).toBe(false);
  expect(mayStartNow(false, true, true)).toBe(false);
});

test("a STOPPED open failure is read as a stop", () => {
  expect(isStopped({ code: "STOPPED", message: "x" })).toBe(true);
  expect(isStopped(new Error("terminal session not found"))).toBe(false);
  expect(isStopped(null)).toBe(false);
});

test("an exit message carries what the exit meant", () => {
  const unable = readSessionMessage(
    JSON.stringify({ type: "exit", exitCode: 1, outcome: { kind: "unable_to_resume", reason: "exited_early", exitCode: 1 } }),
  );
  expect(unable).toEqual({
    kind: "exit",
    exitCode: 1,
    outcome: { kind: "unable_to_resume", reason: "exited_early", exitCode: 1 },
  });
  const plain = readSessionMessage(JSON.stringify({ type: "exit", exitCode: 0, outcome: { kind: "ended" } }));
  expect(plain).toEqual({ kind: "exit", exitCode: 0, outcome: { kind: "ended" } });
  // An exit without an outcome (an older daemon) is an ordinary end.
  expect(readSessionMessage(JSON.stringify({ type: "exit", exitCode: 2 }))).toEqual({
    kind: "exit",
    exitCode: 2,
    outcome: { kind: "ended" },
  });
});

test("the ended banner says what happened and offers the right actions", () => {
  expect(exitBanner({ exitCode: 0, outcome: { kind: "ended" } })).toEqual({
    text: "Session ended.",
    actions: ["start-again"],
  });
  expect(exitBanner({ exitCode: 3, outcome: { kind: "ended" } }).text).toBe("Session ended (exit 3).");
  const early = exitBanner({
    exitCode: 1,
    outcome: { kind: "unable_to_resume", reason: "exited_early", exitCode: 1 },
  });
  expect(early.text).toContain("Couldn't resume the saved conversation (exit 1)");
  expect(early.actions).toEqual(["retry", "start-fresh"]);
  const missingOutcome = { kind: "unable_to_resume", reason: "transcript_missing", exitCode: 1 } as const;
  const missing = { ...exitBanner({ exitCode: 1, outcome: missingOutcome }), outcome: missingOutcome };
  expect(missing.text).toContain("transcript wasn't found");
  expect(missing.actions).toEqual(["retry", "start-fresh"]);
  // Where its harness can list its saved conversations, the person can
  // choose another one to resume.
  expect(exitBanner({ exitCode: 1, outcome: missing.outcome }, { canChoose: true }).actions).toEqual([
    "retry",
    "choose-another",
    "start-fresh",
  ]);
  expect(exitBanner({ exitCode: 0, outcome: { kind: "ended" } }, { canChoose: true }).actions).toEqual(["start-again"]);
});

// A launch that failed before any process existed (a program not installed,
// the harness refusing) is shown where "Session ended" is, with its reason
// and a retry — never a blank pane.
test("a failed launch reads as an ended pane whose banner gives the reason and a retry", () => {
  const failed = failedLaunch(new Error("`codex` isn't installed where oxplow looks"));
  expect(failed).toEqual({
    exitCode: null,
    outcome: { kind: "launch_failed", message: "`codex` isn't installed where oxplow looks" },
  });
  expect(exitBanner(failed)).toEqual({
    text: "Couldn't start the session: `codex` isn't installed where oxplow looks",
    actions: ["retry"],
  });
  // An error that isn't an Error (an IPC rejection) still says something.
  expect(failedLaunch("boom").outcome).toEqual({ kind: "launch_failed", message: "boom" });
  expect(failedLaunch({ message: "refused", code: "INVALID" }).outcome).toEqual({
    kind: "launch_failed",
    message: "refused",
  });
});
