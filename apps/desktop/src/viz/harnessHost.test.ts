import { expect, test } from "bun:test";

import type { PageRun } from "../tauri-bridge/generated/bindings.js";
import { runHarness, type Op, type Server, type Start } from "./harnessHost.js";

const run = {
  page: { id: "acme/board", view: { id: "board", kind: "component", props: { mood: "calm" } }, rail: null },
  params: { stream_id: 1 },
  loaders: {
    items: {
      result: { columns: ["title"], rows: [["Fix it"]], truncated: false, reads: { models: [], tables: [] }, freshness: [] },
      error: null,
      key: "items",
    },
  },
} as unknown as PageRun;

/** A frame whose window takes the `init` and answers on its port like a
 *  component would, with the server's ops scripted. */
function scene(ops: Op[]) {
  const calls: { kind: string; value: unknown }[] = [];
  const results: unknown[] = [];
  const framed: { init?: Record<string, unknown>; port?: MessagePort; seen: unknown[] } = { seen: [] };
  const listeners: (() => void)[] = [];
  const frame = {
    addEventListener: (_: string, cb: () => void) => listeners.push(cb),
    set src(_: string) {
      queueMicrotask(() => listeners.forEach((l) => l()));
    },
    contentWindow: {
      postMessage: (message: Record<string, unknown>, _: string, ports: MessagePort[]) => {
        framed.init = message;
        framed.port = ports[0];
        framed.port.onmessage = (e) => framed.seen.push(e.data);
        framed.port.postMessage({ type: "ready" });
        framed.port.postMessage({ id: "t", method: "setTitle", title: "Board" });
        framed.port.postMessage({ id: "i", method: "invoke", command: "acme.go", input: { n: 1 } });
      },
    },
  } as unknown as HTMLIFrameElement;
  const server: Server = {
    start: async () => ({ version: "v1", run, node: "board", page: null, view: null }) as Start,
    rpc: async (method, body) => {
      calls.push({ kind: `rpc ${method}`, value: body });
      return { done: true };
    },
    record: (kind, value) => calls.push({ kind, value }),
    next: async (result) => {
      results.push(result);
      // Let the frame's messages land before the next op.
      await new Promise((r) => setTimeout(r, 20));
      return ops.shift() ?? { op: "done" };
    },
  };
  const sent: unknown[] = [];
  const agent = {
    send: (id: number, action: unknown) => {
      sent.push(action);
      queueMicrotask(() => answer?.(id, { count: 2 }));
    },
    answers: (cb: (id: number, result: unknown) => void) => {
      answer = cb;
    },
  };
  let answer: ((id: number, result: unknown) => void) | undefined;
  return { frame, server, agent, calls, results, framed, sent };
}

test("the harness mounts the bundle with the page's rows and speaks the real bridge", async () => {
  const s = scene([{ op: "press", id: "fit" }, { op: "dom", action: { query: ".card" } }]);
  await runHarness(s.server, s.frame, s.agent);
  // The frame was fed what the app feeds a component node.
  expect(s.framed.init?.type).toBe("init");
  expect((s.framed.init?.data as Record<string, { rows: unknown[] }>).items.rows).toEqual([["Fix it"]]);
  expect(s.framed.init?.props).toEqual({ mood: "calm" });
  // What only the shell answers is recorded; what the core answers is asked.
  expect(s.calls).toContainEqual({ kind: "ready", value: true });
  expect(s.calls).toContainEqual({ kind: "title", value: "Board" });
  expect(s.calls).toContainEqual({ kind: "rpc invoke", value: { command: "acme.go", input: { n: 1 }, confirmed: false } });
  expect(s.framed.seen).toContainEqual({ id: "i", ok: true, result: { done: true } });
  // A press is the nav bar's; a DOM step is the agent's, its answer the result.
  expect(s.framed.seen).toContainEqual({ type: "action", id: "fit" });
  expect(s.sent).toEqual([{ query: ".card" }]);
  expect(s.results).toEqual([null, null, { count: 2 }]);
});
