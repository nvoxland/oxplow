import { test, expect } from "bun:test";
import { normalize } from "./events.js";

test("SessionStart → session-start with cwd + sessionId", () => {
  const e = normalize("SessionStart", { session_id: "s1", cwd: "/tmp/x" }, 1000);
  expect(e).toEqual({
    kind: "session-start",
    t: 1000,
    sessionId: "s1",
    cwd: "/tmp/x",
  });
});

test("SessionEnd → session-end, tolerates reason or exit_reason", () => {
  expect(normalize("SessionEnd", { session_id: "s", reason: "clear" }, 1).reason).toBe("clear");
  expect(normalize("SessionEnd", { session_id: "s", exit_reason: "logout" }, 1).reason).toBe("logout");
  expect(normalize("SessionEnd", { session_id: "s" }, 1).reason).toBeUndefined();
});

test("UserPromptSubmit → user-prompt carries prompt text", () => {
  const e = normalize("UserPromptSubmit", { session_id: "s", prompt: "hi there" }, 1);
  expect(e.kind).toBe("user-prompt");
  if (e.kind === "user-prompt") expect(e.prompt).toBe("hi there");
});

test("PreToolUse extracts file_path target", () => {
  const e = normalize(
    "PreToolUse",
    { session_id: "s", tool_name: "Read", tool_input: { file_path: "/a.txt" } },
    1,
  );
  expect(e.kind).toBe("tool-use-start");
  if (e.kind === "tool-use-start") {
    expect(e.toolName).toBe("Read");
    expect(e.target).toBe("/a.txt");
  }
});

test("PreToolUse extracts bash command target, truncated", () => {
  const longCmd = "echo " + "x".repeat(200);
  const e = normalize(
    "PreToolUse",
    { session_id: "s", tool_name: "Bash", tool_input: { command: longCmd } },
    1,
  );
  if (e.kind === "tool-use-start") {
    expect(e.target!.length).toBeLessThanOrEqual(80);
    expect(e.target!.endsWith("…")).toBe(true);
  }
});

test("PreToolUse extracts pattern for Grep", () => {
  const e = normalize(
    "PreToolUse",
    { session_id: "s", tool_name: "Grep", tool_input: { pattern: "TODO", path: "/src" } },
    1,
  );
  if (e.kind === "tool-use-start") expect(e.target).toBe("TODO");
});

test("PostToolUse → tool-use-end with ok/error status", () => {
  const ok = normalize(
    "PostToolUse",
    { session_id: "s", tool_name: "Read", tool_response: { file: "..." } },
    1,
  );
  if (ok.kind === "tool-use-end") expect(ok.status).toBe("ok");

  const errByField = normalize(
    "PostToolUse",
    { session_id: "s", tool_name: "Read", tool_response: { error: "nope" } },
    1,
  );
  if (errByField.kind === "tool-use-end") expect(errByField.status).toBe("error");

  const errByFlag = normalize(
    "PostToolUse",
    { session_id: "s", tool_name: "Read", tool_response: { is_error: true } },
    1,
  );
  if (errByFlag.kind === "tool-use-end") expect(errByFlag.status).toBe("error");
});

test("Stop → stop", () => {
  expect(normalize("Stop", { session_id: "s" }, 5).kind).toBe("stop");
});

test("Notification → notification with message", () => {
  const e = normalize("Notification", { session_id: "s", message: "hey" }, 1);
  expect(e.kind).toBe("notification");
  if (e.kind === "notification") expect(e.message).toBe("hey");
});

test("unknown event name → meta", () => {
  const e = normalize("MysteryEvent" as any, { session_id: "s", anything: 1 }, 1);
  expect(e.kind).toBe("meta");
  if (e.kind === "meta") {
    expect(e.hookEventName).toBe("MysteryEvent");
    expect(e.raw).toEqual({ session_id: "s", anything: 1 });
  }
});

test("null/malformed payload does not throw", () => {
  expect(() => normalize("UserPromptSubmit", null as any, 1)).not.toThrow();
  expect(() => normalize("PreToolUse", undefined as any, 1)).not.toThrow();
});
