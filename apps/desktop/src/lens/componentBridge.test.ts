import { expect, test } from "bun:test";

import { IpcCallError } from "../ipc-error.js";
import type { LensRun } from "../tauri-bridge/generated/bindings.js";
import { componentBundleUrl, componentMayOpen, componentRefusal, createBridgeHost, initMessage, lensData, parseFrameMessage, tokensFromStyle, type HostedData } from "./componentBridge.js";

// P6b.D4: a custom component talks to the host over a MessageChannel —
// three requests (query, invoke, navigate) and a `ready`; everything else
// is refused.

test("frame messages parse into the three requests and ready; anything else is refused", () => {
  expect(parseFrameMessage({ type: "ready" })).toEqual({ type: "ready" });
  expect(parseFrameMessage({ id: "1", method: "query", asset: "open-tasks", params: { n: 1, s: "x", z: null } })).toEqual({
    type: "request",
    request: { id: "1", method: "query", asset: "open-tasks", params: { n: 1, s: "x", z: null } },
  });
  expect(parseFrameMessage({ id: "2", method: "invoke", command: "oxplow.work_item.transition", input: { to: "done" } })).toEqual({
    type: "request",
    request: { id: "2", method: "invoke", command: "oxplow.work_item.transition", input: { to: "done" } },
  });
  expect(parseFrameMessage({ id: "3", method: "navigate", uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" })).toEqual({
    type: "request",
    request: { id: "3", method: "navigate", uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", newTab: false, replace: false },
  });
  expect(parseFrameMessage({ id: "4", method: "navigate", uri: "oxplow:oxplow/commit?sha=abc", newTab: true })).toEqual({
    type: "request",
    request: { id: "4", method: "navigate", uri: "oxplow:oxplow/commit?sha=abc", newTab: true, replace: false },
  });
  expect(parseFrameMessage({ id: "5", method: "navigate", uri: "oxplow:oxplow/commit?sha=abc", replace: true })).toEqual({
    type: "request",
    request: { id: "5", method: "navigate", uri: "oxplow:oxplow/commit?sha=abc", newTab: false, replace: true },
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

// tsk984: a frame is served the bundle snapshot its host loaded, by its
// version — a folder of its own, so the bundle's relative URLs stay in it.
test("the bundle URL is the loaded version's folder", () => {
  expect(componentBundleUrl("http://127.0.0.1:7420", "ab12")).toBe("http://127.0.0.1:7420/components/v/ab12/");
  expect(componentBundleUrl("http://127.0.0.1:7420", "a/b")).toBe("http://127.0.0.1:7420/components/v/a%2Fb/");
});

test("theme tokens come from the root's custom properties", () => {
  const style = {
    length: 3,
    item: (i: number) => ["--text-primary", "color", "--accent"][i]!,
    getPropertyValue: (n: string) => ({ "--text-primary": " #eee ", color: "red", "--accent": "#08f" })[n] ?? "",
  };
  const tokens = tokensFromStyle(style);
  expect(tokens).toEqual({ "--text-primary": "#eee", "--accent": "#08f" });
});

test("theme tokens carry the root's color-scheme, so the frame paints no opaque canvas", () => {
  const style = {
    length: 1,
    item: () => "--accent",
    getPropertyValue: (n: string) => ({ "--accent": "#08f", "color-scheme": " dark " })[n] ?? "",
  };
  expect(tokensFromStyle(style)).toEqual({ "--accent": "#08f", "color-scheme": "dark" });
});

// `init` carries the theme's tokens; the kit is a stylesheet the bundle
// links from the daemon. Dropping `kitCss` made the protocol 2;
// `navigate`'s `newTab` made it 3; an error's `field`, 4; a component as a
// page (`page`, a null `run`) and the calls a page makes, 5; a host's view
// (`view`), apart from the page, 8; the view as the URI's `#` and
// `navigate`'s `replace`, 9.
// 12: `init` and `update` carry the page's data — each loader's rows by
// name, its params and props — and `update` says which loaders changed; a
// page shows an inline page under it (`showView`) in place of a lens spec.
test("init carries the run, the data, params, props and tokens, at protocol 12", () => {
  const r = { lens: { id: "x/b", custom: { component: "b", props: { a: 1 } } }, params: { n: 2 }, result: { columns: ["k"], rows: [[1]], truncated: false } } as unknown as LensRun;
  expect(initMessage(lensData(r), { "--accent": "#08f" })).toEqual({
    type: "init",
    protocol: 12,
    run: r,
    data: { rows: { columns: ["k"], rows: [[1]], truncated: false } },
    params: { n: 2 },
    props: { a: 1 },
    page: null,
    view: null,
    tokens: { "--accent": "#08f" },
  });
});

test("a component shown as a page gets its page, its view and the page's data, and no run", () => {
  const page = { streamId: "str1", threadId: "thr2" };
  const view = { viewParams: { from: "2026-10-01" }, historyState: { scroll: 3 } };
  const data: HostedData = { run: null, data: { items: { columns: ["id"], rows: [["a"]], truncated: false } }, params: { from: null }, props: { size: 3 } };
  expect(initMessage(data, {}, page, view)).toEqual({
    type: "init",
    protocol: 12,
    run: null,
    data: data.data,
    params: { from: null },
    props: { size: 3 },
    page,
    view,
    tokens: {},
  });
});

// A replacement or a lens page's component: a lens's run, and its host's view.
test("a component shown by a lens may have its host's view", () => {
  const r = { lens: { id: "x/b" }, params: {}, result: { columns: [], rows: [], truncated: false } } as unknown as LensRun;
  const view = { viewParams: { q: "fix" }, historyState: {} };
  expect(initMessage(lensData(r), {}, null, view)).toMatchObject({ run: r, page: null, view });
});

test("a page's calls parse: setTitle, setActions, openForm and querySql", () => {
  expect(parseFrameMessage({ id: "1", method: "setTitle", title: "Oct 1 – 7" })).toEqual({
    type: "request",
    request: { id: "1", method: "setTitle", title: "Oct 1 – 7" },
  });
  expect(
    parseFrameMessage({
      id: "2",
      method: "setActions",
      actions: [
        { id: "today", label: "Today", primary: true },
        { id: "fit", label: "Fit" },
        { id: "zoom", label: "Zoom", prominent: true, icon: "maximize-2" },
        { id: "csv", label: "As CSV", submenu: " Download ", icon: "unicorn" },
      ],
    }),
  ).toEqual({
    type: "request",
    request: {
      id: "2",
      method: "setActions",
      actions: [
        { id: "today", label: "Today", primary: true, prominent: false, disabled: false },
        { id: "fit", label: "Fit", primary: false, prominent: false, disabled: false },
        { id: "zoom", label: "Zoom", primary: false, prominent: true, disabled: false, icon: "maximize-2" },
        { id: "csv", label: "As CSV", primary: false, prominent: false, disabled: false, submenu: "Download" },
      ],
    },
  });
  expect(parseFrameMessage({ id: "3", method: "openForm", command: "acme.item.create" })).toEqual({
    type: "request",
    request: { id: "3", method: "openForm", command: "acme.item.create", input: {} },
  });
  expect(parseFrameMessage({ id: "4", method: "querySql", sql: "SELECT ?1", params: [1] })).toEqual({
    type: "request",
    request: { id: "4", method: "querySql", sql: "SELECT ?1", params: [1] },
  });
  for (const bad of [
    { id: "5", method: "setTitle", title: 7 },
    { id: "6", method: "setActions", actions: [{ id: "x" }] },
    { id: "7", method: "querySql", sql: "SELECT 1", params: [{}] },
    { id: "8", method: "openForm" },
  ]) {
    expect(parseFrameMessage(bad)).toBeNull();
  }
});

const run = { lens: { id: "x/burn" }, params: {}, result: { columns: [], rows: [], truncated: false } } as unknown as LensRun;
const sent = lensData(run);

/** A host over a real channel; what the frame side receives. */
function harness(deps: Partial<Parameters<typeof createBridgeHost>[1]> = {}) {
  const channel = new MessageChannel();
  const received: unknown[] = [];
  channel.port2.onmessage = (e) => received.push(e.data);
  const calls: unknown[][] = [];
  const host = createBridgeHost(channel.port1, {
    query: async (asset, params) => {
      calls.push(["query", asset, params]);
      return run;
    },
    invoke: async (command, input, confirmed) => {
      calls.push(["invoke", command, input, confirmed]);
      if (!confirmed) throw new IpcCallError("needs confirmation", "NEEDS_CONFIRMATION");
      return { result: null, audit_id: 1, event_id: null, inverse: null };
    },
    navigate: (ref, opts) => {
      calls.push(["navigate", ref, opts]);
      return true;
    },
    confirm: async () => true,
    onReady: () => calls.push(["ready"]),
    ...deps,
  }, sent);
  const send = (data: unknown) => channel.port2.postMessage(data);
  const settle = () => new Promise((r) => setTimeout(r, 20));
  return { received, calls, send, settle, channel, host };
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
  yes.send({ id: "i1", method: "invoke", command: "oxplow.work_item.transition", input: { to: "done" } });
  await yes.settle();
  expect(yes.calls).toEqual([
    ["invoke", "oxplow.work_item.transition", { to: "done" }, false],
    ["invoke", "oxplow.work_item.transition", { to: "done" }, true],
  ]);
  expect(yes.received).toEqual([{ id: "i1", ok: true, result: null }]);
  yes.channel.port1.close();

  const no = harness({ confirm: async () => false });
  no.send({ id: "i2", method: "invoke", command: "oxplow.work_item.transition", input: {} });
  await no.settle();
  expect(no.received).toEqual([{ id: "i2", ok: false, error: { code: "CANCELLED", message: "The person didn't confirm `oxplow.work_item.transition`." } }]);
  no.channel.port1.close();
});

test("navigate opens the ref; a malformed request is answered with an error when it has an id", async () => {
  const h = harness();
  h.send({ id: "n1", method: "navigate", uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" });
  h.send({ id: "n2", method: "navigate", uri: "oxplow:oxplow/commit?sha=abc", newTab: true });
  h.send({ id: "b1", method: "exec" });
  await h.settle();
  expect(h.calls).toEqual([
    ["navigate", "oxplow:oxplow/work_item?id=tsk1&provider=oxplow", { newTab: false, replace: false }],
    ["navigate", "oxplow:oxplow/commit?sha=abc", { newTab: true, replace: false }],
  ]);
  expect(h.received).toEqual([
    { id: "n1", ok: true, result: null },
    { id: "n2", ok: true, result: null },
    { id: "b1", ok: false, error: { code: "BAD_REQUEST", message: "Not a request this host knows." } },
  ]);
  h.channel.port1.close();
});

test("ready is heard once; an update posts only what the frame hasn't seen, naming the loaders that changed", async () => {
  const h = harness();
  const page: HostedData = {
    run: null,
    data: { a: { columns: ["n"], rows: [[1]], truncated: false }, b: { columns: ["m"], rows: [], truncated: false } },
    params: { p: 1 },
    props: null,
  };
  const host = createBridgeHost(h.channel.port1, {
    query: async () => run,
    invoke: async () => ({ result: null, audit_id: 1, event_id: null, inverse: null }),
    navigate: () => {},
    confirm: async () => true,
    onReady: () => h.calls.push(["ready"]),
  }, page);
  h.send({ type: "ready" });
  h.send({ type: "ready" });
  await h.settle();
  expect(h.calls).toEqual([["ready"]]);
  host.update({ ...page, data: { ...page.data } });
  await h.settle();
  expect(h.received).toEqual([], "the data init carried");
  const next: HostedData = { ...page, data: { ...page.data, b: { columns: ["m"], rows: [[2]], truncated: false } } };
  host.update(next);
  host.update({ ...next });
  await h.settle();
  expect(h.received).toEqual([{ type: "update", run: null, data: next.data, params: { p: 1 }, changed: ["b"] }]);
  // New params alone: an update naming no loader.
  host.update({ ...next, params: { p: 2 } });
  await h.settle();
  expect(h.received.at(-1)).toEqual({ type: "update", run: null, data: next.data, params: { p: 2 }, changed: [] });
  h.channel.port1.close();
});

test("a component navigates to app pages only", async () => {
  expect(componentMayOpen("oxplow:oxplow/work_item?id=tsk1&provider=oxplow")).toBe(true);
  expect(componentMayOpen("oxplow:oxplow/handoff?id=hnd1")).toBe(true);
  expect(componentMayOpen("https://evil.example/?d=1")).toBe(false);
  expect(componentMayOpen("not a ref")).toBe(false);
  const h = harness({ navigate: (ref) => componentMayOpen(ref) });
  h.send({ id: "n2", method: "navigate", uri: "https://evil.example/" });
  await h.settle();
  expect(h.received).toEqual([
    { id: "n2", ok: false, error: { code: "INVALID", message: "A component may open oxplow's pages only." } },
  ]);
  h.channel.port1.close();
});

// tsk960: a refused invoke — a component that is not allowed, or a policy saying
// no — is shown by the host, beside the frame, whatever the component's
// own code does with the reply; any other failure is the frame's to show.
test("a page's title and actions reach the host, and a press reaches the frame", async () => {
  const titles: string[] = [];
  const actions: unknown[] = [];
  const h = harness({ setTitle: (t) => titles.push(t), setActions: (a) => actions.push(a) });
  h.send({ id: "t", method: "setTitle", title: "Oct" });
  h.send({ id: "a", method: "setActions", actions: [{ id: "fit", label: "Fit" }] });
  await h.settle();
  expect(titles).toEqual(["Oct"]);
  expect(actions).toEqual([[{ id: "fit", label: "Fit", primary: false, prominent: false, disabled: false }]]);
  expect(h.received).toEqual([
    { id: "t", ok: true, result: null },
    { id: "a", ok: true, result: null },
  ]);
  h.host.action("fit");
  await h.settle();
  expect(h.received.at(-1)).toEqual({ type: "action", id: "fit" });
  h.channel.port1.close();
});

// Each kind of page state by the name of its scope: a page's view params
// (its tab's address) and its history state (this step of its tab), set
// through the host and heard when Back or Forward changes them.
test("a page sets its view params and history state through the host, and hears them change", async () => {
  const views: unknown[] = [];
  const states: unknown[] = [];
  const h = harness({ setViewParams: (p) => views.push(p), setHistoryState: (p) => states.push(p) });
  h.send({ id: "p", method: "setViewParams", params: { from: "2026-10-01", to: null } });
  h.send({ id: "q", method: "setViewParams", params: { from: 3 } });
  h.send({ id: "s", method: "setHistoryState", patch: { scroll: 40, open: ["a"] } });
  h.send({ id: "t", method: "setHistoryState", patch: [1] });
  await h.settle();
  expect(views).toEqual([{ from: "2026-10-01", to: null }]);
  expect(states).toEqual([{ scroll: 40, open: ["a"] }]);
  expect(h.received.map((m) => {
    const r = m as { ok: boolean; error?: { code: string } };
    return r.ok ? true : r.error!.code;
  })).toEqual([
    true,
    "BAD_REQUEST",
    true,
    "BAD_REQUEST",
  ]);
  h.received.length = 0;
  h.host.viewParams({ from: "2026-10-01" });
  h.host.viewParams({ from: "2026-10-01" });
  h.host.viewParams({ from: "2026-10-08" });
  h.host.historyState({ scroll: 40 });
  h.host.historyState({ scroll: 40 });
  await h.settle();
  expect(h.received).toEqual([
    { type: "viewParams", params: { from: "2026-10-01" } },
    { type: "viewParams", params: { from: "2026-10-08" } },
    { type: "historyState", state: { scroll: 40 } },
  ]);
  h.channel.port1.close();
});

// Page memory is the component's own, for the person, in every tab of it.
test("a component reads and writes its page memory through the host; a refused write says why", async () => {
  const kept = new Map<string, unknown>([["layout", { cols: 3 }]]);
  const h = harness({
    pageMemoryGet: async (key) => kept.get(key) ?? null,
    pageMemorySet: (key, value) => {
      if (JSON.stringify(value).length > 10) throw Object.assign(new Error("too big"), { code: "INVALID" });
      kept.set(key, value);
    },
  });
  h.send({ id: "g", method: "pageMemoryGet", key: "layout" });
  h.send({ id: "s", method: "pageMemorySet", key: "n", value: 2 });
  h.send({ id: "b", method: "pageMemorySet", key: "n", value: "far too long a value" });
  h.send({ id: "x", method: "pageMemoryGet", key: "" });
  await h.settle();
  expect(h.received).toEqual([
    { id: "g", ok: true, result: { cols: 3 } },
    { id: "s", ok: true, result: null },
    { id: "b", ok: false, error: { code: "INVALID", message: "too big" } },
    { id: "x", ok: false, error: { code: "BAD_REQUEST", message: "Not a request this host knows." } },
  ]);
  expect(kept.get("n")).toBe(2);
  h.channel.port1.close();
});

test("shown by a lens, a component sets no page title, actions or form", async () => {
  const h = harness();
  h.send({ id: "t", method: "setTitle", title: "Oct" });
  h.send({ id: "a", method: "setActions", actions: [] });
  h.send({ id: "f", method: "openForm", command: "x.y.z" });
  h.send({ id: "s", method: "showView", spec: null });
  h.send({ id: "o", method: "openForm", form: { title: "T", schema: { type: "object" }, values: {} } });
  h.send({ id: "i", method: "signIn", instance: "t/x", name: "TOKEN" });
  await h.settle();
  expect(h.received.map((m) => (m as { error?: { message: string } }).error?.message)).toEqual([
    "Only a component shown as a page sets its title, actions or the view it shows.",
    "Only a component shown as a page sets its title, actions or the view it shows.",
    "This host shows no forms and signs in for no component.",
    "Only a component shown as a page sets its title, actions or the view it shows.",
    "This host shows no forms and signs in for no component.",
    "This host shows no forms and signs in for no component.",
  ]);
  h.channel.port1.close();
});

// A tile, a slot, the rail: its host gives it no view.
test("a host with no view refuses view params and history state", async () => {
  const h = harness();
  h.send({ id: "p", method: "setViewParams", params: {} });
  h.send({ id: "s", method: "setHistoryState", patch: {} });
  await h.settle();
  expect(h.received).toEqual([
    { id: "p", ok: false, error: { code: "INVALID", message: "This host gives the component no view: only a page, a replacement or a lens's own page does." } },
    { id: "s", ok: false, error: { code: "INVALID", message: "This host gives the component no view: only a page, a replacement or a lens's own page does." } },
  ]);
  h.channel.port1.close();
});

// A replacement shares only the keys its target's contract lists.
test("a view param the host refuses says why", async () => {
  const h = harness({
    setViewParams: () => {
      throw Object.assign(new Error("`sha` isn't in vcs.history.graph's view (q, author, branch)."), { code: "INVALID" });
    },
  });
  h.send({ id: "p", method: "setViewParams", params: { sha: "abc" } });
  await h.settle();
  expect(h.received).toEqual([
    { id: "p", ok: false, error: { code: "INVALID", message: "`sha` isn't in vcs.history.graph's view (q, author, branch)." } },
  ]);
  h.channel.port1.close();
});

test("querySql and openForm answer with the host's result; a closed form is CANCELLED", async () => {
  const sql = { columns: ["n"], rows: [[1]], truncated: false };
  const h = harness({
    querySql: async () => sql as never,
    openForm: async (command) => (command === "acme.ok" ? { result: { made: 1 } } : null),
  });
  h.send({ id: "q", method: "querySql", sql: "SELECT 1", params: [] });
  h.send({ id: "f", method: "openForm", command: "acme.ok", input: { a: 1 } });
  h.send({ id: "g", method: "openForm", command: "acme.closed" });
  await h.settle();
  expect(h.received).toEqual([
    { id: "q", ok: true, result: sql },
    { id: "f", ok: true, result: { made: 1 } },
    { id: "g", ok: false, error: { code: "CANCELLED", message: "The person closed the form for `acme.closed`." } },
  ]);
  h.channel.port1.close();
});

test("componentRefusal is a refused invoke's reason, and nothing else's", () => {
  const reason = "component `x/burn` runs `…` and is not allowed to call this command.";
  expect(componentRefusal(new IpcCallError(reason, "DENIED"))).toBe(reason);
  expect(componentRefusal(new IpcCallError("bad input", "INVALID"))).toBeNull();
  expect(componentRefusal(new Error("boom"))).toBeNull();
});

// A page shows an inline page the host draws (`showView`, protocol 12),
// and says how tall its content is.
test("showView and size parse; anything else in their place doesn't", () => {
  const spec = "title: Explore\ndata: { rows: { sql: SELECT 1 AS n } }\nview: { viz: number, from: rows }\n";
  expect(parseFrameMessage({ id: "1", method: "showView", spec })).toEqual({ type: "request", request: { id: "1", method: "showView", spec } });
  expect(parseFrameMessage({ id: "2", method: "showView", spec: null })).toEqual({ type: "request", request: { id: "2", method: "showView", spec: null } });
  expect(parseFrameMessage({ id: "3", method: "showView", spec: { title: "E" } })).toBeNull();
  expect(parseFrameMessage({ id: "4", method: "showSpec", spec: null })).toBeNull();
  expect(parseFrameMessage({ type: "size", height: 320 })).toEqual({ type: "size", height: 320 });
  expect(parseFrameMessage({ type: "size", height: -1 })).toBeNull();
  expect(parseFrameMessage({ type: "size", height: "tall" })).toBeNull();
});

test("a page's showView answers its first loader's rows once drawn, or the host's refusal with its field; size reaches the host", async () => {
  const shown: unknown[] = [];
  const heights: number[] = [];
  const h = harness({
    showView: async (spec) => {
      if (spec?.includes("bad")) throw Object.assign(new Error("no such table: nope"), { code: "INVALID", field: "/data/rows/sql" });
      shown.push(spec);
      return spec ? { columns: ["n"], rows: [[1]], truncated: false } : null;
    },
    size: (height) => heights.push(height),
  });
  h.send({ id: "a", method: "showView", spec: "good" });
  h.send({ id: "b", method: "showView", spec: "bad" });
  h.send({ id: "c", method: "showView", spec: null });
  h.send({ type: "size", height: 240 });
  await h.settle();
  expect(h.received).toEqual([
    { id: "a", ok: true, result: { columns: ["n"], rows: [[1]], truncated: false } },
    { id: "b", ok: false, error: { code: "INVALID", message: "no such table: nope", field: "/data/rows/sql" } },
    { id: "c", ok: true, result: null },
  ]);
  expect(shown).toEqual(["good", null]);
  expect(heights).toEqual([240]);
  h.channel.port1.close();
});

test("openForm with a form of its own, signIn and cancelSignIn parse; anything else in their place doesn't", () => {
  const form = { title: "Configure", schema: { type: "object" }, values: { team: "core" } };
  expect(parseFrameMessage({ id: "1", method: "openForm", form })).toEqual({ type: "request", request: { id: "1", method: "openForm", form } });
  expect(parseFrameMessage({ id: "2", method: "openForm", form: { title: "T", schema: { type: "object" } } })).toEqual({
    type: "request",
    request: { id: "2", method: "openForm", form: { title: "T", schema: { type: "object" }, values: {} } },
  });
  expect(parseFrameMessage({ id: "3", method: "openForm", form: { title: "T" } })).toBeNull();
  expect(parseFrameMessage({ id: "4", method: "openForm", form: { title: "T", schema: [] } })).toBeNull();
  expect(parseFrameMessage({ id: "5", method: "signIn", instance: "t/x", name: "TOKEN" })).toEqual({
    type: "request",
    request: { id: "5", method: "signIn", instance: "t/x", name: "TOKEN" },
  });
  expect(parseFrameMessage({ id: "6", method: "cancelSignIn", instance: "t/x", name: "TOKEN" })).toEqual({
    type: "request",
    request: { id: "6", method: "cancelSignIn", instance: "t/x", name: "TOKEN" },
  });
  expect(parseFrameMessage({ id: "7", method: "signIn", instance: "t/x" })).toBeNull();
});
