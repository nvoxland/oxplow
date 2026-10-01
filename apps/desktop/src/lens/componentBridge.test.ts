import { expect, test } from "bun:test";

import { IpcCallError } from "../ipc-error.js";
import type { LensRun } from "../tauri-bridge/generated/bindings.js";
import { componentBundleUrl, componentNavigationTarget, createBridgeHost, kitCss, parseFrameMessage, tokensFromStyle } from "./componentBridge.js";

// P6b.D4: a custom component talks to the host over a MessageChannel —
// three requests (query, invoke, navigate) and a `ready`; everything else
// is refused.

test("frame messages parse into the three requests and ready; anything else is refused", () => {
  expect(parseFrameMessage({ type: "ready" })).toEqual({ type: "ready" });
  expect(parseFrameMessage({ id: "1", method: "query", asset: "open-tasks", params: { n: 1, s: "x", z: null } })).toEqual({
    type: "request",
    request: { id: "1", method: "query", asset: "open-tasks", params: { n: 1, s: "x", z: null } },
  });
  expect(parseFrameMessage({ id: "2", method: "invoke", command: "work_item.transition", input: { to: "done" } })).toEqual({
    type: "request",
    request: { id: "2", method: "invoke", command: "work_item.transition", input: { to: "done" } },
  });
  expect(parseFrameMessage({ id: "3", method: "navigate", ref: "work_item:oxplow:tsk1" })).toEqual({
    type: "request",
    request: { id: "3", method: "navigate", ref: "work_item:oxplow:tsk1" },
  });
  for (const bad of [
    null,
    "query",
    { method: "query", asset: "a", params: {} },
    { id: "1", method: "exec", command: "x" },
    { id: "1", method: "query", asset: "a", params: { n: { nested: true } } },
    { id: "1", method: "query", params: {} },
    { id: "1", method: "invoke", input: {} },
  ]) {
    expect(parseFrameMessage(bad)).toBeNull();
  }
});

test("the bundle URL carries the stream as its first segment, so relative URLs stay in its worktree", () => {
  expect(componentBundleUrl("http://127.0.0.1:7420", "my ext", "burn/down", "str 2")).toBe(
    "http://127.0.0.1:7420/components/str%202/my%20ext/burn%2Fdown/",
  );
  expect(componentBundleUrl("http://127.0.0.1:7420", "x", "c", null)).toBe("http://127.0.0.1:7420/components/primary/x/c/");
});

test("theme tokens come from the root's custom properties", () => {
  const style = {
    length: 3,
    item: (i: number) => ["--text-primary", "color", "--accent"][i]!,
    getPropertyValue: (n: string) => ({ "--text-primary": " #eee ", color: "red", "--accent": "#08f" })[n] ?? "",
  };
  const tokens = tokensFromStyle(style);
  expect(tokens).toEqual({ "--text-primary": "#eee", "--accent": "#08f" });
  expect(kitCss(tokens)).toContain(":root { --text-primary: #eee; --accent: #08f; }");
});

const run = { lens: { id: "x/burn" }, params: {}, result: { columns: [], rows: [], truncated: false } } as unknown as LensRun;

/** A host over a real channel; what the frame side receives. */
function harness(deps: Partial<Parameters<typeof createBridgeHost>[1]> = {}) {
  const channel = new MessageChannel();
  const received: unknown[] = [];
  channel.port2.onmessage = (e) => received.push(e.data);
  const calls: unknown[][] = [];
  createBridgeHost(channel.port1, {
    query: async (asset, params) => {
      calls.push(["query", asset, params]);
      return run;
    },
    invoke: async (command, input, confirmed) => {
      calls.push(["invoke", command, input, confirmed]);
      if (!confirmed) throw new IpcCallError("needs confirmation", "NEEDS_CONFIRMATION");
      return { result: null, audit_id: 1, event_id: null, inverse: null };
    },
    navigate: (ref) => {
      calls.push(["navigate", ref]);
      return true;
    },
    confirm: async () => true,
    onReady: () => calls.push(["ready"]),
    ...deps,
  }, run);
  const send = (data: unknown) => channel.port2.postMessage(data);
  const settle = () => new Promise((r) => setTimeout(r, 20));
  return { received, calls, send, settle, channel };
}

test("a query runs the declared lens through the host and answers ok", async () => {
  const h = harness();
  h.send({ type: "ready" });
  h.send({ id: "q1", method: "query", asset: "open-tasks", params: { n: 1 } });
  await h.settle();
  expect(h.calls).toEqual([["ready"], ["query", "open-tasks", { n: 1 }]]);
  expect(h.received).toEqual([{ id: "q1", ok: true, result: run }]);
  h.channel.port1.close();
});

test("an invoke that asks is confirmed in the host, then runs confirmed; declined, it's CANCELLED", async () => {
  const yes = harness();
  yes.send({ id: "i1", method: "invoke", command: "work_item.transition", input: { to: "done" } });
  await yes.settle();
  expect(yes.calls).toEqual([
    ["invoke", "work_item.transition", { to: "done" }, false],
    ["invoke", "work_item.transition", { to: "done" }, true],
  ]);
  expect(yes.received).toEqual([{ id: "i1", ok: true, result: null }]);
  yes.channel.port1.close();

  const no = harness({ confirm: async () => false });
  no.send({ id: "i2", method: "invoke", command: "work_item.transition", input: {} });
  await no.settle();
  expect(no.received).toEqual([{ id: "i2", ok: false, error: { code: "CANCELLED", message: "The person didn't confirm `work_item.transition`." } }]);
  no.channel.port1.close();
});

test("navigate opens the ref; a malformed request is answered with an error when it has an id", async () => {
  const h = harness();
  h.send({ id: "n1", method: "navigate", ref: "work_item:oxplow:tsk1" });
  h.send({ id: "b1", method: "exec" });
  await h.settle();
  expect(h.calls).toEqual([["navigate", "work_item:oxplow:tsk1"]]);
  expect(h.received).toEqual([
    { id: "n1", ok: true, result: null },
    { id: "b1", ok: false, error: { code: "BAD_REQUEST", message: "Not a query, invoke or navigate request." } },
  ]);
  h.channel.port1.close();
});

test("ready is heard once; an update posts only a result the frame hasn't seen", async () => {
  const h = harness();
  const host = createBridgeHost(h.channel.port1, {
    query: async () => run,
    invoke: async () => ({ result: null, audit_id: 1, event_id: null, inverse: null }),
    navigate: () => {},
    confirm: async () => true,
    onReady: () => h.calls.push(["ready"]),
  }, run);
  h.send({ type: "ready" });
  h.send({ type: "ready" });
  await h.settle();
  expect(h.calls).toEqual([["ready"]]);
  host.update({ ...run });
  await h.settle();
  expect(h.received).toEqual([], "the run init carried");
  const next = { ...run, result: { ...run.result, rows: [[1]] } } as LensRun;
  host.update(next);
  host.update({ ...next });
  await h.settle();
  expect(h.received).toEqual([{ type: "update", run: next }]);
  h.channel.port1.close();
});

// A component reaches no network, so it may not open an outside URL by
// navigating the host: only app pages, and a refused navigate is answered.
test("a component navigates to app pages only", async () => {
  expect(componentNavigationTarget("work_item:oxplow:tsk1")?.kind).toBe("work_item");
  expect(componentNavigationTarget("page:external-url?url=https%3A%2F%2Fevil.example%2F%3Fd%3D1")).toBeNull();
  expect(componentNavigationTarget("not a ref")).toBeNull();
  const h = harness({ navigate: (ref) => componentNavigationTarget(ref) !== null });
  h.send({ id: "n2", method: "navigate", ref: "page:external-url?url=https%3A%2F%2Fevil.example%2F" });
  await h.settle();
  expect(h.received).toEqual([
    { id: "n2", ok: false, error: { code: "INVALID", message: "A component may open oxplow's pages only." } },
  ]);
  h.channel.port1.close();
});
