import { expect, test } from "bun:test";

import { startClientHost, type ClientHostDeps, type ClientHandlers } from "./clientHost.js";
import type { OxplowEvent } from "./tauri-bridge/generated/bindings.js";

// The window as a command host: it says which capabilities it hosts, does
// what the daemon calls it for, and answers — in the caller's thread.

function harness(handlers: ClientHandlers) {
  const registered: string[][] = [];
  const answers: Array<[string, unknown]> = [];
  let emit: (e: OxplowEvent) => void = () => {};
  let reconnect: () => void = () => {};
  const deps: ClientHostDeps = {
    register: async (caps) => void registered.push(caps),
    answer: async (id, a) => void answers.push([id, a]),
    subscribe: (fn) => {
      emit = fn;
      return () => {};
    },
    onReconnect: (fn) => {
      reconnect = fn;
      return () => {};
    },
  };
  const stop = startClientHost(() => handlers, deps);
  return { registered, answers, emit: (e: OxplowEvent) => emit(e), reconnect: () => reconnect(), stop };
}

const call = (op: string, input: unknown, threadId: string | null = "thr3"): OxplowEvent =>
  ({ kind: "clientCall", id: `c-${op}`, threadId, actor: "agent:thr3", capability: "tabs.write", op, input }) as OxplowEvent;

test("it registers what it hosts, again on reconnect", () => {
  const h = harness({ "tabs.write": { open: () => null } });
  expect(h.registered).toEqual([["tabs.write"]]);
  h.reconnect();
  expect(h.registered).toEqual([["tabs.write"], ["tabs.write"]]);
});

test("a call runs its handler in the caller's thread and answers with its result or error", async () => {
  const seen: unknown[] = [];
  const h = harness({
    "tabs.write": {
      open: (input, ctx) => {
        seen.push([input, ctx.threadId, ctx.actor]);
        return { open: true };
      },
      close: () => {
        throw new Error("no tab `file:x`");
      },
    },
  });
  h.emit(call("open", { ref: "file:a.rs" }));
  h.emit(call("close", { ref: "file:x" }));
  h.emit(call("pin", {}));
  h.emit({ kind: "configChanged" } as OxplowEvent);
  await new Promise((r) => setTimeout(r, 0));
  expect(seen).toEqual([[{ ref: "file:a.rs" }, "thr3", "agent:thr3"]]);
  expect([...h.answers].sort((a, b) => a[0].localeCompare(b[0]))).toEqual([
    ["c-close", { error: "no tab `file:x`" }],
    ["c-open", { result: { open: true } }],
    ["c-pin", { error: "the window doesn't do `tabs.write` `pin`" }],
  ]);
});
