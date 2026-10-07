import { expect, test } from "bun:test";

import type { LensRun } from "../tauri-bridge/generated/bindings.js";
import { BRIDGE_PROTOCOL, createBridgeHost, initMessage, type BridgeDeps } from "./componentBridge.js";
// The library the daemon serves to component bundles (P9.A4): the one
// place it and the host are proven to speak the same protocol. It is a
// classic script — loading it defines the global `oxplow`.
await import("../../../../crates/oxplow-daemon/assets/oxplow-component.js");
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const { connect, PROTOCOL } = (globalThis as any).oxplow;

const run = (n: number) =>
  ({ lens: { id: "x/burn" }, params: {}, result: { columns: ["n"], rows: [[n]], truncated: false } }) as unknown as LensRun;

/** A frame's window (where `init` arrives) and the host on the other end
 *  of its channel, as `CustomComponentViz` wires them. */
function harness(deps: Partial<BridgeDeps> = {}, protocol: unknown = BRIDGE_PROTOCOL) {
  const channel = new MessageChannel();
  let readies = 0;
  const host = createBridgeHost(
    channel.port1,
    {
      query: async (asset, params) => ({ ...run(7), lens: { id: asset }, params }) as unknown as LensRun,
      invoke: async () => ({ result: { done: true } }) as never,
      navigate: () => true,
      confirm: async () => true,
      onReady: () => {
        readies += 1;
      },
      ...deps,
    },
    run(1),
  );
  const frame = new EventTarget();
  // The `init` the host really sends (`CustomComponentViz` posts this
  // builder's message), so the library is proven against it (tsk856).
  const first = { ...run(1), lens: { id: "x/burn", custom: { component: "burn", props: { color: "accent" } } } } as unknown as LensRun;
  const init = () =>
    frame.dispatchEvent(
      Object.assign(new Event("message"), {
        data: { ...initMessage(first, { "--accent": "#08f" }), protocol },
        ports: [channel.port2],
      }),
    );
  return { frame, init, host, readies: () => readies, close: () => channel.port1.close() };
}
const settle = () => new Promise((r) => setTimeout(r, 10));

test("the library and the host speak the same protocol", () => {
  expect(PROTOCOL).toBe(BRIDGE_PROTOCOL);
});

test("connect resolves with what init carried and says ready once", async () => {
  const h = harness();
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  expect(c.run.result.rows).toEqual([[1]]);
  expect(c.props).toEqual({ color: "accent" });
  expect(c.tokens).toEqual({ "--accent": "#08f" });
  // The kit is a stylesheet the bundle links (tsk961), not text in `init`.
  expect("kitCss" in c).toBe(false);
  expect(c.protocol).toBe(BRIDGE_PROTOCOL);
  await settle();
  expect(h.readies()).toBe(1);
  h.close();
});

// tsk961: the theme reaches the frame as custom properties set on its
// root through the CSSOM — the kit's sheet reads them, and no inline
// <style> is needed.
test("applyTheme sets each token on the document's root", async () => {
  const h = harness();
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  const set: Array<[string, string]> = [];
  const doc = { documentElement: { style: { setProperty: (k: string, v: string) => set.push([k, v]) } } };
  c.applyTheme(doc);
  expect(set).toEqual([["--accent", "#08f"]]);
  expect("applyKitCss" in c).toBe(false);
  h.close();
});

test("query, invoke and navigate resolve with the host's replies", async () => {
  const h = harness();
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  const answer = await c.query("open-tasks", { n: 1 });
  expect(answer.lens.id).toBe("open-tasks");
  expect(answer.params).toEqual({ n: 1 });
  expect(await c.invoke("oxplow.work_item.transition", { to: "done" })).toEqual({ done: true });
  expect(await c.navigate("work_item:oxplow:tsk1")).toBeNull();
  h.close();
});

test("a refused request rejects with the host's code and message", async () => {
  const h = harness({
    invoke: async () => {
      throw Object.assign(new Error("no"), { code: "DENIED" });
    },
    navigate: () => false,
  });
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  await expect(c.invoke("oxplow.vcs.commit", {})).rejects.toEqual({ code: "DENIED", message: "no" });
  await expect(c.navigate("https://example.com")).rejects.toMatchObject({ code: "INVALID" });
  h.close();
});

test("onUpdate hears a re-run until it unsubscribes", async () => {
  const h = harness();
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  const seen: unknown[] = [];
  const off = c.onUpdate((r: LensRun) => seen.push(r.result.rows));
  h.host.update(run(2));
  await settle();
  expect(seen).toEqual([[[2]]]);
  off();
  h.host.update(run(3));
  await settle();
  expect(seen).toEqual([[[2]]]);
  h.close();
});

test("a host speaking another protocol is refused, by number", async () => {
  const other = BRIDGE_PROTOCOL + 1;
  const h = harness({}, other);
  const connecting = connect({ target: h.frame });
  h.init();
  await expect(connecting).rejects.toThrow(new RegExp(`protocol ${other}`));
  await settle();
  expect(h.readies()).toBe(0);
  h.close();
});

test("connect gives up when no init arrives", async () => {
  await expect(connect({ target: new EventTarget(), timeoutMs: 20 })).rejects.toThrow(/init/);
});
