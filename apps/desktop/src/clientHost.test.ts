import { expect, test } from "bun:test";

import { startClientHost, type ClientHostDeps, type ClientHandlers } from "./clientHost.js";
import type { OxplowEvent } from "./tauri-bridge/generated/bindings.js";

// The window as a command host: it says which scopes it hosts, does
// what the daemon calls it for, and answers — in the caller's thread. It
// hosts under its window id (its socket's and tab reports'): calls are
// addressed to one window, and it says when it closes.

// The window hosts commands as the window it is everywhere else (its event
// socket, its tab reports, its logs), so a call can be addressed to the
// window an agent reads about; and it needs no secure context to start,
// since a window on plain HTTP is still a window.
test("the window hosts as its own window id, with no secure context", async () => {
  const { getUiClientId } = await import("./uiClientId.js");
  const registered: string[] = [];
  const crypto = globalThis.crypto as { randomUUID?: unknown };
  const randomUUID = crypto.randomUUID;
  // A page on plain HTTP has no `crypto.randomUUID`.
  Object.defineProperty(crypto, "randomUUID", { value: undefined, configurable: true, writable: true });
  expect(crypto.randomUUID).toBeUndefined();
  try {
    const stop = startClientHost(() => ({}), {
      register: async (client) => void registered.push(client),
      unregister: async () => {},
      answer: async () => {},
      subscribe: () => () => {},
      onReconnect: () => () => {},
      onClose: () => () => {},
    });
    await Promise.resolve();
    stop();
  } finally {
    Object.defineProperty(crypto, "randomUUID", { value: randomUUID, configurable: true, writable: true });
  }
  expect(registered).toEqual([getUiClientId()]);
});

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
        throw new Error("no tab `oxplow:oxplow/file?path=x`");
      },
    },
  });
  h.emit(call("open", { uri: "oxplow:oxplow/file?path=a.rs" }));
  h.emit(call("close", { uri: "oxplow:oxplow/file?path=x" }));
  h.emit(call("pin", {}));
  h.emit({ kind: "configChanged" } as OxplowEvent);
  await settle();
  // The handler knows the call it answers: what it runs on the daemon for
  // it runs as the call's actor.
  expect(seen).toEqual([[{ uri: "oxplow:oxplow/file?path=a.rs" }, "thr3", "agent:thr3", { client: "w1", id: "c-open" }]]);
  expect([...h.answers].sort((a, b) => a[1].localeCompare(b[1]))).toEqual([
    ["w1", "c-close", { error: "no tab `oxplow:oxplow/file?path=x`" }],
    ["w1", "c-open", { result: { open: true } }],
    ["w1", "c-pin", { error: "the window doesn't do `tabs.write` `pin`" }],
  ]);
});

test("a call addressed to another window is left to it", async () => {
  const seen: unknown[] = [];
  const h = harness({ "tabs.write": { open: (input) => void seen.push(input) } });
  h.emit(call("open", { uri: "oxplow:oxplow/file?path=a.rs" }, "w2"));
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

// A caller that waits for the window (the browser suite, a link opened at
// boot) waits for this: a call before it is refused "no window is open".
test("it says it is registered once the daemon has taken its first registration, and only then", async () => {
  let acknowledge: () => void = () => {};
  const registered: string[] = [];
  const deps: ClientHostDeps = {
    register: (client) => new Promise<void>((r) => (acknowledge = () => (registered.push(client), r()))),
    unregister: async () => {},
    answer: async () => {},
    subscribe: () => () => {},
    onReconnect: () => () => {},
    onClose: () => () => {},
  };
  let told = 0;
  const stop = startClientHost(() => ({ "tabs.write": {} }), deps, "w1", () => (told += 1));
  await settle();
  expect(told).toBe(0);
  acknowledge();
  await settle();
  expect(registered).toEqual(["w1"]);
  expect(told).toBe(1);
  stop();
});
