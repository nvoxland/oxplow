import { test, expect } from "bun:test";
import { ingestHookPayload, HookEventStore } from "./hook-ingest.js";

test("ingestHookPayload stores normalized events in a ring buffer", () => {
  const store = new HookEventStore(4);
  ingestHookPayload(store, "UserPromptSubmit", { prompt: "hi", session_id: "s1" }, { streamId: "stream-a" });
  ingestHookPayload(store, "PreToolUse", {
    tool_name: "Read",
    tool_input: { file_path: "/a.txt" },
    session_id: "s1",
  }, { streamId: "stream-a", pane: "working" });
  const events = store.list("stream-a");
  expect(events).toHaveLength(2);
  expect(events[0].streamId).toBe("stream-a");
  expect(events[0].normalized.kind).toBe("user-prompt");
  if (events[0].normalized.kind === "user-prompt") {
    expect(events[0].normalized.prompt).toBe("hi");
  }
  expect(events[1].normalized.kind).toBe("tool-use-start");
  expect(events[1].pane).toBe("working");
  if (events[1].normalized.kind === "tool-use-start") {
    expect(events[1].normalized.toolName).toBe("Read");
    expect(events[1].normalized.target).toBe("/a.txt");
  }
});

test("ring buffer drops oldest when capacity is exceeded", () => {
  const store = new HookEventStore(2);
  ingestHookPayload(store, "Notification", { message: "1" }, { streamId: "stream-a" });
  ingestHookPayload(store, "Notification", { message: "2" }, { streamId: "stream-a" });
  ingestHookPayload(store, "Notification", { message: "3" }, { streamId: "stream-a" });
  const events = store.list("stream-a");
  expect(events).toHaveLength(2);
  const msgs = events.map((e) => (e.normalized.kind === "notification" ? e.normalized.message : ""));
  expect(msgs).toEqual(["2", "3"]);
});

test("each event has monotonic id and a timestamp in the normalized payload", () => {
  const store = new HookEventStore(10);
  ingestHookPayload(store, "Stop", {}, { streamId: "stream-a" });
  ingestHookPayload(store, "Stop", {}, { streamId: "stream-a" });
  const [a, b] = store.list("stream-a");
  expect(b.id).toBeGreaterThan(a.id);
  expect(typeof a.normalized.t).toBe("number");
});

test("subscribers receive events as they are ingested", () => {
  const store = new HookEventStore(10);
  const received: string[] = [];
  const unsub = store.subscribe((e) => received.push(`${e.streamId}:${e.normalized.kind}`), "stream-a");
  ingestHookPayload(store, "UserPromptSubmit", { prompt: "a" }, { streamId: "stream-a" });
  ingestHookPayload(store, "Stop", {}, { streamId: "stream-b" });
  ingestHookPayload(store, "Stop", {}, { streamId: "stream-a" });
  unsub();
  ingestHookPayload(store, "Stop", {}, { streamId: "stream-a" }); // not received
  expect(received).toEqual(["stream-a:user-prompt", "stream-a:stop"]);
});
