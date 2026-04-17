import { describe, expect, test } from "bun:test";
import { deriveBatchAgentStatus } from "./agent-status.js";
import type { StoredEvent } from "./hook-ingest.js";
import type { NormalizedEvent } from "../core/events.js";

let nextId = 1;

function event(normalized: NormalizedEvent): StoredEvent {
  return { id: nextId++, streamId: "s-1", batchId: "b-1", normalized };
}

describe("deriveBatchAgentStatus", () => {
  test("empty history is idle", () => {
    expect(deriveBatchAgentStatus([])).toBe("idle");
  });

  test("session-start alone is done (ready, nothing happening)", () => {
    expect(deriveBatchAgentStatus([event({ kind: "session-start", t: 0 })])).toBe("done");
  });

  test("user prompt flips to working", () => {
    expect(deriveBatchAgentStatus([
      event({ kind: "session-start", t: 0 }),
      event({ kind: "user-prompt", t: 1, prompt: "hi" }),
    ])).toBe("working");
  });

  test("tool call keeps working", () => {
    expect(deriveBatchAgentStatus([
      event({ kind: "user-prompt", t: 0, prompt: "hi" }),
      event({ kind: "tool-use-start", t: 1, toolName: "Read" }),
      event({ kind: "tool-use-end", t: 2, toolName: "Read", status: "ok" }),
    ])).toBe("working");
  });

  test("stop after work is done", () => {
    expect(deriveBatchAgentStatus([
      event({ kind: "user-prompt", t: 0, prompt: "hi" }),
      event({ kind: "tool-use-end", t: 1, toolName: "Read", status: "ok" }),
      event({ kind: "stop", t: 2 }),
    ])).toBe("done");
  });

  test("notification during work is waiting", () => {
    expect(deriveBatchAgentStatus([
      event({ kind: "user-prompt", t: 0, prompt: "hi" }),
      event({ kind: "tool-use-start", t: 1, toolName: "Bash" }),
      event({ kind: "notification", t: 2, message: "permission required" }),
    ])).toBe("waiting");
  });

  test("notification resolved by subsequent tool returns to working", () => {
    expect(deriveBatchAgentStatus([
      event({ kind: "notification", t: 0, message: "permission required" }),
      event({ kind: "tool-use-end", t: 1, toolName: "Bash", status: "ok" }),
    ])).toBe("working");
  });

  test("session-end is idle regardless of prior activity", () => {
    expect(deriveBatchAgentStatus([
      event({ kind: "user-prompt", t: 0, prompt: "hi" }),
      event({ kind: "tool-use-start", t: 1, toolName: "Read" }),
      event({ kind: "session-end", t: 2, reason: "exit" }),
    ])).toBe("idle");
  });

  test("meta events do not change status", () => {
    expect(deriveBatchAgentStatus([
      event({ kind: "user-prompt", t: 0, prompt: "hi" }),
      event({ kind: "meta", t: 1, hookEventName: "SubagentStart", raw: {} }),
    ])).toBe("working");
  });
});
