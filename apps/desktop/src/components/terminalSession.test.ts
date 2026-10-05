import { expect, test } from "bun:test";

import { isSessionGone, readSessionMessage, reopened } from "./terminalSession.js";

// tsk1026: after the daemon restarted, the agent pane kept the dead session's
// screen and swallowed keystrokes; an agent that exited said nothing.

test("an exit message is read, with its code", () => {
  expect(readSessionMessage(JSON.stringify({ type: "exit", exitCode: 0 }))).toEqual({ kind: "exit", exitCode: 0 });
  expect(readSessionMessage(JSON.stringify({ type: "exit" }))).toEqual({ kind: "exit", exitCode: null });
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
