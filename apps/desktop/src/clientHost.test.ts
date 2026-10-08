import { expect, test } from "bun:test";

import { startClientHost, type ClientHostDeps, type ClientHandlers } from "./clientHost.js";
import type { OxplowEvent } from "./tauri-bridge/generated/bindings.js";

// The window as a command host: it says which scopes it hosts, does
// what the daemon calls it for, and answers — in the caller's thread. It
// has an id of its own: calls are addressed to one window, and it says
// when it closes.

function harness(handlers: ClientHandlers) {
  const registered: Array<[string, string[]]> = [];
  const unregistered: string[] = [];
  const answers: Array<[string, string, unknown]> = [];
  let emit: (e: OxplowEvent) => void = () => {};
  let reconnect: () => void = () => {};
  let close: () => void = () => {};
  const deps: ClientHostDeps = {
    register: async (client, caps) => void registered.push([client, caps]),
    unregister: async (client) => void unregistered.push(client),
    answer: async (client, id, a) => void answers.push([client, id, a]),
    subscribe: (fn) => {
      emit = fn;
      return () => {};
    },
    onReconnect: (fn) => {
      reconnect = fn;
      return () => {};
    },
    onClose: (fn) => {
      close = fn;
      return () => {};
    },
  };
  const stop = startClientHost(() => handlers, deps, "w1");
  return {
    registered,
    unregistered,
    answers,
    emit: (e: OxplowEvent) => emit(e),
    reconnect: () => reconnect(),
    close: () => close(),
    stop,
  };
}

const call = (op: string, input: unknown, client = "w1"): OxplowEvent =>
  ({ kind: "clientCall", id: `c-${op}`, client, threadId: "thr3", actor: "agent:thr3", scope: "tabs.write", op, input }) as OxplowEvent;

const settle = () => new Promise((r) => setTimeout(r, 0));

test("it registers what it hosts under its id, again on reconnect", () => {
  const h = harness({ "tabs.write": { open: () => null } });
  expect(h.registered).toEqual([["w1", ["tabs.write"]]]);
  h.reconnect();
  expect(h.registered).toEqual([
    ["w1", ["tabs.write"]],
    ["w1", ["tabs.write"]],
  ]);
});

test("a call runs its handler in the caller's thread and answers with its result or error", async () => {
  const seen: unknown[] = [];
  const h = harness({
    "tabs.write": {
      open: (input, ctx) => {
        seen.push([input, ctx.threadId, ctx.actor, ctx.call]);
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
  await settle();
  // The handler knows the call it answers: what it runs on the daemon for
  // it runs as the call's actor.
  expect(seen).toEqual([[{ ref: "file:a.rs" }, "thr3", "agent:thr3", { client: "w1", id: "c-open" }]]);
  expect([...h.answers].sort((a, b) => a[1].localeCompare(b[1]))).toEqual([
    ["w1", "c-close", { error: "no tab `file:x`" }],
    ["w1", "c-open", { result: { open: true } }],
    ["w1", "c-pin", { error: "the window doesn't do `tabs.write` `pin`" }],
  ]);
});

test("a call addressed to another window is left to it", async () => {
  const seen: unknown[] = [];
  const h = harness({ "tabs.write": { open: (input) => void seen.push(input) } });
  h.emit(call("open", { ref: "file:a.rs" }, "w2"));
  await settle();
  expect(seen).toEqual([]);
  expect(h.answers).toEqual([]);
});

test("it unregisters when the window closes or the host stops", () => {
  const h = harness({ "tabs.write": { open: () => null } });
  h.close();
  expect(h.unregistered).toEqual(["w1"]);
  h.stop();
  expect(h.unregistered).toEqual(["w1", "w1"]);
});
