import { describe, expect, test } from "bun:test";
import { deriveThreadAgentStatus } from "./agent-status.js";
import type { StoredEvent } from "./hook-ingest.js";
import type { NormalizedEvent } from "../core/events.js";

let nextId = 1;

function event(normalized: NormalizedEvent): StoredEvent {
  return { id: nextId++, streamId: "s-1", threadId: "b-1", normalized };
}

describe("deriveThreadAgentStatus", () => {
  test("empty history is idle", () => {
    expect(deriveThreadAgentStatus([])).toBe("idle");
  });

  test("session-start alone is done (ready, nothing happening)", () => {
    expect(deriveThreadAgentStatus([event({ kind: "session-start", t: 0 })])).toBe("done");
  });

  test("user prompt flips to working", () => {
    expect(deriveThreadAgentStatus([
      event({ kind: "session-start", t: 0 }),
      event({ kind: "user-prompt", t: 1, prompt: "hi" }),
    ])).toBe("working");
  });

  test("tool call keeps working", () => {
    expect(deriveThreadAgentStatus([
      event({ kind: "user-prompt", t: 0, prompt: "hi" }),
      event({ kind: "tool-use-start", t: 1, toolName: "Read" }),
      event({ kind: "tool-use-end", t: 2, toolName: "Read", status: "ok" }),
    ])).toBe("working");
  });

  test("stop after work is done", () => {
    expect(deriveThreadAgentStatus([
      event({ kind: "user-prompt", t: 0, prompt: "hi" }),
      event({ kind: "tool-use-end", t: 1, toolName: "Read", status: "ok" }),
      event({ kind: "stop", t: 2 }),
    ])).toBe("done");
  });

  test("notification during work is waiting", () => {
    expect(deriveThreadAgentStatus([
      event({ kind: "user-prompt", t: 0, prompt: "hi" }),
      event({ kind: "tool-use-start", t: 1, toolName: "Bash" }),
      event({ kind: "notification", t: 2, message: "permission required" }),
    ])).toBe("waiting");
  });

  test("notification resolved by subsequent tool returns to working", () => {
    expect(deriveThreadAgentStatus([
      event({ kind: "notification", t: 0, message: "permission required" }),
      event({ kind: "tool-use-end", t: 1, toolName: "Bash", status: "ok" }),
    ])).toBe("working");
  });

  test("session-end is idle regardless of prior activity", () => {
    expect(deriveThreadAgentStatus([
      event({ kind: "user-prompt", t: 0, prompt: "hi" }),
      event({ kind: "tool-use-start", t: 1, toolName: "Read" }),
      event({ kind: "session-end", t: 2, reason: "exit" }),
    ])).toBe("idle");
  });

  test("meta events do not change status", () => {
    expect(deriveThreadAgentStatus([
      event({ kind: "user-prompt", t: 0, prompt: "hi" }),
      event({ kind: "meta", t: 1, hookEventName: "SubagentStart", raw: {} }),
    ])).toBe("working");
  });

  test("stop while a Task subagent is still in flight stays working", () => {
    // Parent dispatches a Task subagent (PreToolUse fires). Before the
    // subagent's PostToolUse returns, a Stop hook arrives (subagent or
    // parent pause). Tab-icon should reflect "still doing work" until
    // the Task tool actually returns.
    expect(deriveThreadAgentStatus([
      event({ kind: "user-prompt", t: 0, prompt: "hi" }),
      event({ kind: "tool-use-start", t: 1, toolName: "Task" }),
      event({ kind: "stop", t: 2 }),
    ])).toBe("working");
  });

  test("stop after Task subagent returns is done", () => {
    expect(deriveThreadAgentStatus([
      event({ kind: "user-prompt", t: 0, prompt: "hi" }),
      event({ kind: "tool-use-start", t: 1, toolName: "Task" }),
      event({ kind: "tool-use-end", t: 2, toolName: "Task", status: "ok" }),
      event({ kind: "stop", t: 3 }),
    ])).toBe("done");
  });

  test("multiple Task subagents in flight: stop stays working until all return", () => {
    expect(deriveThreadAgentStatus([
      event({ kind: "user-prompt", t: 0, prompt: "hi" }),
      event({ kind: "tool-use-start", t: 1, toolName: "Task" }),
      event({ kind: "tool-use-start", t: 2, toolName: "Task" }),
      event({ kind: "tool-use-end", t: 3, toolName: "Task", status: "ok" }),
      event({ kind: "stop", t: 4 }),
    ])).toBe("working");
  });
});
