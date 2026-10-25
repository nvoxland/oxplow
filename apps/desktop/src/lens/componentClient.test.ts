import { expect, test } from "bun:test";

import type { LensRun } from "../tauri-bridge/generated/bindings.js";
import { BRIDGE_PROTOCOL, createBridgeHost, initMessage, lensData, type BridgeDeps, type HostedData } from "./componentBridge.js";
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
    lensData(run(1)),
  );
  const frame = new EventTarget();
  // The `init` the host really sends (`CustomComponentViz` posts this
  // builder's message), so the library is proven against it (tsk856).
  const first = { ...run(1), lens: { id: "x/burn", custom: { component: "burn", props: { color: "accent" } } } } as unknown as LensRun;
  const init = () =>
    frame.dispatchEvent(
      Object.assign(new Event("message"), {
        data: { ...initMessage(lensData(first), { "--accent": "#08f" }), protocol },
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
  expect(c.data).toEqual({ rows: { columns: ["n"], rows: [[1]], truncated: false } });
  expect(c.params).toEqual({});
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
  expect(await c.navigate("oxplow:oxplow/work_item?id=tsk1&provider=oxplow")).toBeNull();
  h.close();
});

test("navigate can ask for a new tab", async () => {
  const opened: Array<[string, unknown]> = [];
  const h = harness({
    navigate: (ref, opts) => {
      opened.push([ref, opts]);
      return true;
    },
  });
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  await c.navigate("oxplow:oxplow/commit?sha=abc", { newTab: true });
  await c.navigate("oxplow:oxplow/commit?sha=def");
  expect(opened).toEqual([
    ["oxplow:oxplow/commit?sha=abc", { newTab: true, replace: false }],
    ["oxplow:oxplow/commit?sha=def", { newTab: false, replace: false }],
  ]);
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

// Protocol 4: a command refusing invalid input names the field, so a
// component's form marks it.
test("an invalid input's refusal names its field", async () => {
  const h = harness({
    invoke: async () => {
      throw Object.assign(new Error("a title can't be empty"), { code: "INVALID", field: "/title" });
    },
  });
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  await expect(c.invoke("oxplow.work_item.update", { uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", title: "" })).rejects.toEqual({
    code: "INVALID",
    message: "a title can't be empty",
    field: "/title",
  });
  h.close();
});

test("onUpdate hears new data until it unsubscribes, with the loaders that changed", async () => {
  const h = harness();
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  const seen: unknown[] = [];
  const off = c.onUpdate((u: { run: LensRun; data: HostedData["data"]; changed: string[] }) => seen.push([u.run.result.rows, u.data.rows?.rows, u.changed]));
  h.host.update(lensData(run(2)));
  await settle();
  expect(seen).toEqual([[[[2]], [[2]], ["rows"]]]);
  expect(c.run.result.rows).toEqual([[2]]);
  expect(c.data.rows.rows).toEqual([[2]]);
  off();
  h.host.update(lensData(run(3)));
  await settle();
  expect(seen.length).toBe(1);
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

/** A component shown as a page: its `init` has the page and no run, and
 *  the host answers the page's own calls. */
const pageData: HostedData = { run: null, data: { items: { columns: ["id"], rows: [["a"]], truncated: false } }, params: { from: "2026-10-01" }, props: { size: 2 } };

function pageHarness(deps: Partial<BridgeDeps> = {}) {
  const view = { viewParams: { from: "2026-10-01" }, historyState: { scroll: 3 } };
  const channel = new MessageChannel();
  const titles: string[] = [];
  const host = createBridgeHost(
    channel.port1,
    {
      query: async () => run(0),
      invoke: async () => ({ result: null }) as never,
      navigate: () => true,
      confirm: async () => true,
      onReady: () => {},
      setTitle: (t) => titles.push(t),
      setActions: () => {},
      querySql: async (sql, params) => ({ columns: ["sql", "n"], rows: [[sql, params[0] ?? null]], truncated: false }) as never,
      openForm: async () => ({ result: { made: true } }),
      ...deps,
    },
    pageData,
    view,
  );
  const frame = new EventTarget();
  const page = { streamId: "str1", threadId: null };
  const init = () =>
    frame.dispatchEvent(Object.assign(new Event("message"), { data: initMessage(pageData, {}, page, view), ports: [channel.port2] }));
  return { frame, init, host, titles, close: () => channel.port1.close() };
}

test("a page's component gets its page, runs its own SQL, opens a form and sets its title", async () => {
  const h = pageHarness();
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  expect(c.run).toBeNull();
  expect(c.data).toEqual(pageData.data);
  expect(c.params).toEqual({ from: "2026-10-01" });
  expect(c.props).toEqual({ size: 2 });
  expect(c.page).toEqual({ streamId: "str1", threadId: null });
  expect(c.viewParams).toEqual({ from: "2026-10-01" });
  expect(c.historyState).toEqual({ scroll: 3 });
  expect(await c.querySql("SELECT ?1", [3])).toEqual({ columns: ["sql", "n"], rows: [["SELECT ?1", 3]], truncated: false });
  expect(await c.openForm("acme.item.create", { title: "x" })).toEqual({ made: true });
  await c.setTitle("October");
  await c.setActions([{ id: "fit", label: "Fit" }]);
  expect(h.titles).toEqual(["October"]);
  h.close();
});

test("a page's component sets its view params and history state, and hears them change", async () => {
  const views: unknown[] = [];
  const states: unknown[] = [];
  const h = pageHarness({ setViewParams: (p) => views.push(p), setHistoryState: (p) => states.push(p) });
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  await c.setViewParams({ from: "2026-10-08", to: null });
  await c.setHistoryState({ scroll: 90 });
  expect(views).toEqual([{ from: "2026-10-08", to: null }]);
  expect(states).toEqual([{ scroll: 90 }]);
  const heardViews: unknown[] = [];
  const heardStates: unknown[] = [];
  const stopViews = c.onViewParams((p: unknown) => heardViews.push(p));
  c.onHistoryState((s: unknown) => heardStates.push(s));
  h.host.viewParams({ from: "2026-10-15" });
  h.host.historyState({ scroll: 5 });
  await settle();
  expect(heardViews).toEqual([{ from: "2026-10-15" }]);
  expect(heardStates).toEqual([{ scroll: 5 }]);
  expect(c.viewParams).toEqual({ from: "2026-10-15" });
  expect(c.historyState).toEqual({ scroll: 5 });
  stopViews();
  h.host.viewParams({ from: "2026-10-22" });
  await settle();
  expect(heardViews).toHaveLength(1);
  h.close();
});

test("a component's page memory reads and writes through the host", async () => {
  const kept = new Map<string, unknown>();
  const h = pageHarness({
    pageMemoryGet: async (key) => kept.get(key) ?? null,
    pageMemorySet: (key, value) => void kept.set(key, value),
  });
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  expect(await c.pageMemory.get("layout")).toBeNull();
  await c.pageMemory.set("layout", { cols: 3 });
  expect(await c.pageMemory.get("layout")).toEqual({ cols: 3 });
  h.close();
});

test("onAction hears each press of a page action", async () => {
  const h = pageHarness();
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  const pressed: string[] = [];
  const stop = c.onAction((id: string) => pressed.push(id));
  h.host.action("fit");
  await settle();
  stop();
  h.host.action("today");
  await settle();
  expect(pressed).toEqual(["fit"]);
  h.close();
});

// A replacement: a lens's run, and the keys of its host's view its
// target's contract lists.
test("a component shown by a lens with its host's view reads, sets and hears it", async () => {
  const channel = new MessageChannel();
  const views: unknown[] = [];
  const view = { viewParams: { q: "fix" }, historyState: {} };
  const host = createBridgeHost(
    channel.port1,
    {
      query: async () => run(0),
      invoke: async () => ({ result: null }) as never,
      navigate: () => true,
      confirm: async () => true,
      onReady: () => {},
      setViewParams: (p) => views.push(p),
      setHistoryState: () => {},
    },
    lensData(run(1)),
    view,
  );
  const frame = new EventTarget();
  const connecting = connect({ target: frame });
  frame.dispatchEvent(Object.assign(new Event("message"), { data: initMessage(lensData(run(1)), {}, null, view), ports: [channel.port2] }));
  const c = await connecting;
  expect(c.run.result.rows).toEqual([[1]]);
  expect(c.page).toBeNull();
  expect(c.viewParams).toEqual({ q: "fix" });
  await c.setViewParams({ q: null });
  expect(views).toEqual([{ q: null }]);
  const heard: unknown[] = [];
  c.onViewParams((p: unknown) => heard.push(p));
  host.viewParams({ author: "ann" });
  await settle();
  expect(heard).toEqual([{ author: "ann" }]);
  channel.port1.close();
});

test("a component shown by a lens with no view has none", async () => {
  const h = harness();
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  expect(c.viewParams).toBeNull();
  expect(c.historyState).toBeNull();
  await expect(c.setViewParams({ q: "x" })).rejects.toMatchObject({ code: "INVALID" });
  h.close();
});

// Protocol 12: a page hands oxplow an inline page to draw under it.
test("showView resolves with its first loader's rows once the host drew it, and rejects with the host's refusal and its field", async () => {
  const shown: unknown[] = [];
  const h = harness({
    showView: async (spec) => {
      if (spec?.includes("bad")) throw Object.assign(new Error("no such table: nope"), { code: "INVALID", field: "/data/rows/sql" });
      shown.push(spec);
      return spec ? { columns: ["n"], rows: [[1]], truncated: false } : null;
    },
  });
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  expect(await c.showView("title: E\ndata: { rows: { sql: SELECT 1 AS n } }\nview: { viz: number, from: rows }\n")).toEqual({ columns: ["n"], rows: [[1]], truncated: false });
  expect(await c.showView(null)).toBeNull();
  expect(await c.showView()).toBeNull();
  const refused = await c.showView("bad").catch((e: unknown) => e);
  expect(refused).toEqual({ code: "INVALID", message: "no such table: nope", field: "/data/rows/sql" });
  expect(shown.length).toBe(3);
  expect("showSpec" in c).toBe(false);
  h.close();
});

// Protocol 11: a page asks the host to sign in, and opens a form of its own.
test("signIn and cancelSignIn reach the host, which says how it ended; openForm({ schema }) answers the submitted values", async () => {
  const asked: unknown[] = [];
  const h = harness({
    signIn: async (instance, name) => {
      asked.push(["signIn", instance, name]);
      return { state: "signed_in" };
    },
    cancelSignIn: async (instance, name) => void asked.push(["cancelSignIn", instance, name]),
    askForm: async (form) => {
      asked.push(["form", form]);
      return { team: "core" };
    },
  });
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  expect(await c.signIn("t/x", "TOKEN")).toEqual({ state: "signed_in" });
  expect(await c.cancelSignIn("t/x", "TOKEN")).toBeNull();
  const schema = { type: "object", properties: { team: { type: "string" } } };
  expect(await c.openForm({ title: "Configure", schema, values: { team: "" } })).toEqual({ team: "core" });
  expect(asked).toEqual([
    ["signIn", "t/x", "TOKEN"],
    ["cancelSignIn", "t/x", "TOKEN"],
    ["form", { title: "Configure", schema, values: { team: "" } }],
  ]);
  h.close();
});

test("a form the person closes rejects CANCELLED", async () => {
  const h = harness({ askForm: async () => null });
  const connecting = connect({ target: h.frame });
  h.init();
  const c = await connecting;
  const closed = await c.openForm({ title: "T", schema: { type: "object" } }).catch((e: unknown) => e);
  expect(closed).toMatchObject({ code: "CANCELLED" });
  h.close();
});
