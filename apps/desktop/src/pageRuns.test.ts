import { describe, expect, test } from "bun:test";

import { CLOSE_AFTER_MS, COALESCE_MS, createPageRuns, type PageRunsTransport, type RunChange } from "./pageRuns.js";
import type { PageRun } from "./tauri-bridge/generated/bindings.js";
import type { PageKey } from "./pageRuns.js";

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

function fakeRun(label: string): PageRun {
  return { page: { title: label } } as unknown as PageRun;
}

function fake() {
  let next = 0;
  const calls = { open: 0, close: [] as string[], fetch: [] as string[][], setParams: 0, rerun: [] as string[], visible: [] as [string, boolean][] };
  let change: (c: RunChange) => void = () => {};
  let reconnect: () => void = () => {};
  const current = new Map<string, string>();
  const sequences = new Map<string, number>();
  let stale = false;
  const transport: PageRunsTransport = {
    open: async (key) => {
      calls.open += 1;
      const runId = `run-${++next}`;
      current.set(runId, key.page ?? "inline");
      return { runId, run: fakeRun(`${key.page}@1`) };
    },
    close: async (runId) => {
      calls.close.push(runId);
    },
    fetch: async (ids) => {
      calls.fetch.push(ids);
      return Object.fromEntries(ids.map((id) => [id, { run: fakeRun(`${current.get(id)}@fetched`), seq: sequences.get(id) ?? 0 }]));
    },
    setParams: async () => {
      calls.setParams += 1;
      return fakeRun("re-run");
    },
    rerun: async (runId) => {
      calls.rerun.push(runId);
      return fakeRun("refreshed");
    },
    setVisible: async (runId, visible) => {
      calls.visible.push([runId, visible]);
      return visible && stale;
    },
    onChange: (h) => {
      change = h;
      return () => {};
    },
    onReconnect: (h) => {
      reconnect = h;
      return () => {};
    },
  };
  return { transport, calls, missChange: () => { stale = true; }, emit: (c: RunChange) => { sequences.set(c.runId, Math.max(c.seq, sequences.get(c.runId) ?? 0)); change(c); }, reconnect: () => reconnect() };
}

const key = (page: string) => ({ page, params: {}, streamId: null, threadId: null });

describe("pageRuns", () => {
  test("a run the daemon forgot is opened again, keeping what is shown until it answers", async () => {
    const f = fake();
    const runs = createPageRuns(f.transport);
    const a = runs.open(key("p"));
    await wait(0);
    const forgotten = a.get().runId!;
    expect(f.calls.open).toBe(1);
    runs.reopen(["run-unknown"]);
    expect(f.calls.open).toBe(1);
    runs.reopen([forgotten]);
    expect(a.get().run).not.toBeNull();
    expect(a.get().stale).toBe(true);
    await wait(0);
    expect(f.calls.open).toBe(2);
    expect(a.get().runId).not.toBe(forgotten);
    expect(a.get().stale).toBe(false);
    expect(runs.runIds()).toEqual([a.get().runId!]);
  });

  test("a run whose hosts are all hidden is paused, and fetched once when one shows", async () => {
    const f = fake();
    const runs = createPageRuns(f.transport);
    const a = runs.open(key("p"));
    const b = runs.open(key("p"));
    await wait(0);
    a.setVisible(false);
    await wait(0);
    expect(f.calls.visible).toEqual([]);
    b.setVisible(false);
    await wait(0);
    expect(f.calls.visible).toEqual([["run-1", false]]);
    f.missChange();
    a.setVisible(true);
    await wait(COALESCE_MS + 20);
    expect(f.calls.visible).toEqual([["run-1", false], ["run-1", true]]);
    expect(f.calls.fetch).toEqual([["run-1"]]);
  });

  test("a host opened on a paused run shows it again: the daemon is told, and the run fetched once", async () => {
    const f = fake();
    const runs = createPageRuns(f.transport);
    const a = runs.open(key("p"));
    await wait(0);
    a.setVisible(false);
    await wait(0);
    expect(f.calls.visible).toEqual([["run-1", false]]);
    f.missChange();
    // The tab's page is drawn again by a new host (the old one went with its
    // tab's content), on the run that is still open and paused.
    runs.open(key("p"));
    await wait(COALESCE_MS + 20);
    expect(f.calls.visible).toEqual([["run-1", false], ["run-1", true]]);
    expect(f.calls.fetch).toEqual([["run-1"]]);
  });

  test("out-of-order fetch replies use their actual snapshot sequences, not their triggering notices", async () => {
    const f = fake();
    const replies: ((value: Record<string, { run: PageRun; seq: number }>) => void)[] = [];
    f.transport.fetch = () => new Promise((resolve) => replies.push(resolve));
    const runs = createPageRuns(f.transport);
    const handle = runs.open(key("acme/x"));
    await wait(0);
    f.emit({ runId: "run-1", seq: 1, loaders: [], tree: true });
    await wait(COALESCE_MS + 20);
    f.emit({ runId: "run-1", seq: 2, loaders: [], tree: true });
    await wait(COALESCE_MS + 20);
    // The first request happened to read a newer server snapshot, while
    // the second request's older snapshot spent longer on the network.
    replies[0]({ "run-1": { run: fakeRun("newest"), seq: 3 } });
    await wait(0);
    replies[1]({ "run-1": { run: fakeRun("older"), seq: 2 } });
    await wait(0);
    expect(handle.get().run?.page.title).toBe("newest");
    f.emit({ runId: "run-1", seq: 3, loaders: [], tree: true });
    await wait(COALESCE_MS + 20);
    expect(replies).toHaveLength(2);
  });

  for (const beforeOperation of [true, false]) test(`a fetch started ${beforeOperation ? "before" : "during"} a direct operation cannot overwrite its completed result`, async () => {
    const f = fake();
    const replies: ((value: Record<string, { run: PageRun; seq: number }>) => void)[] = [];
    f.transport.fetch = () => new Promise((resolve) => replies.push(resolve));
    let finish!: (run: PageRun) => void;
    f.transport.rerun = () => new Promise((resolve) => { finish = resolve; });
    const runs = createPageRuns(f.transport);
    const handle = runs.open(key("acme/x"));
    await wait(0);
    let operation: Promise<void>;
    if (beforeOperation) {
      f.emit({ runId: "run-1", seq: 1, loaders: [], tree: true });
      await wait(COALESCE_MS + 20);
      operation = handle.rerun();
      await wait(0);
    } else {
      operation = handle.rerun();
      await wait(0);
      f.emit({ runId: "run-1", seq: 1, loaders: [], tree: true });
      await wait(COALESCE_MS + 20);
    }
    finish(fakeRun("operation result"));
    await operation;
    replies[0]({ "run-1": { run: fakeRun("precommit snapshot"), seq: 1 } });
    await wait(0);
    expect(handle.get().run?.page.title).toBe("operation result");
    await wait(COALESCE_MS + 20);
    expect(replies).toHaveLength(2);
    replies[1]({ "run-1": { run: fakeRun("authoritative result"), seq: 2 } });
    await wait(0);
    expect(handle.get().run?.page.title).toBe("authoritative result");
  });

  test("a direct operation reply cannot overwrite a newer applied model snapshot", async () => {
    const f = fake();
    let finish!: (run: PageRun) => void;
    f.transport.rerun = () => new Promise((resolve) => { finish = resolve; });
    f.transport.fetch = async () => ({ "run-1": { run: fakeRun("new model snapshot"), seq: 2 } });
    const runs = createPageRuns(f.transport);
    const handle = runs.open(key("acme/x"));
    await wait(0);
    const operation = handle.rerun();
    await wait(0);
    f.emit({ runId: "run-1", seq: 2, loaders: [], tree: true });
    await wait(COALESCE_MS + 20);
    expect(handle.get().run?.page.title).toBe("new model snapshot");
    finish(fakeRun("old operation reply"));
    await operation;
    expect(handle.get().run?.page.title).toBe("new model snapshot");
  });
  test("a delayed refresh from before reconnect cannot replace the reopened run or its action target", async () => {
    const f = fake();
    let finish!: (run: PageRun) => void;
    f.transport.rerun = () => new Promise((resolve) => { finish = resolve; });
    const runs = createPageRuns(f.transport);
    const handle = runs.open(key("acme/x"));
    await wait(0);
    const refresh = handle.rerun();
    await wait(0);
    f.reconnect();
    await wait(0);
    finish(fakeRun("stale refresh"));
    await refresh;
    expect(handle.get().runId).toBe("run-2");
    expect(handle.get().run?.page.title).toBe("acme/x@1");
    expect(runs.runIds()).toEqual(["run-2"]);
  });

  test("a failed old parameter request cannot erase a successful reopened run", async () => {
    const f = fake();
    let fail!: (error: Error) => void;
    f.transport.setParams = () => new Promise((_, reject) => { fail = reject; });
    const runs = createPageRuns(f.transport);
    const handle = runs.open(key("acme/x"));
    await wait(0);
    const change = handle.setParams({ q: "new" });
    await wait(0);
    f.reconnect();
    await wait(0);
    fail(new Error("old connection failed"));
    await change;
    expect(handle.get().runId).toBe("run-2");
    expect(handle.get().error).toBeNull();
    expect(handle.get().stale).toBe(false);
  });

  test("direct parameter changes survive reconnect without changing the shared subscription identity", async () => {
    const f = fake();
    const opened: PageKey[] = [];
    const open = f.transport.open;
    f.transport.open = async (key) => { opened.push(key); return open(key); };
    const runs = createPageRuns(f.transport);
    const initial = { ...key("acme/x"), params: { q: "before", mode: "all" } };
    const first = runs.open(initial);
    await wait(0);
    await first.setParams({ q: "after" });
    const second = runs.open(initial);
    expect(f.calls.open).toBe(1);
    f.reconnect();
    await wait(0);
    expect(opened[1].params).toEqual({ q: "after", mode: "all" });
    expect(first.get().runId).toBe(second.get().runId);
    expect(initial.params).toEqual({ q: "before", mode: "all" });
  });

  test("a notice arriving before the open reply is fetched after the run is registered", async () => {
    const f = fake();
    let finish!: (value: { runId: string; run: PageRun }) => void;
    f.transport.open = () => new Promise((resolve) => { finish = resolve; });
    f.transport.fetch = async (ids) => { f.calls.fetch.push(ids); return { early: { run: fakeRun("fresh"), seq: 2 } }; };
    const runs = createPageRuns(f.transport);
    const handle = runs.open(key("acme/x"));
    f.emit({ runId: "early", seq: 2, loaders: ["n"], tree: false });
    f.emit({ runId: "early", seq: 1, loaders: ["n"], tree: false });
    finish({ runId: "early", run: fakeRun("initial") });
    await wait(COALESCE_MS + 20);
    expect(f.calls.fetch).toEqual([["early"]]);
    expect(handle.get().run?.page.title).toBe("fresh");
    f.emit({ runId: "early", seq: 2, loaders: ["n"], tree: false });
    await wait(COALESCE_MS + 20);
    expect(f.calls.fetch).toHaveLength(1);
  });

  test("overflow of unrelated early notices still reconciles the new run without swallowing its next notice", async () => {
    const f = fake();
    let finish!: (value: { runId: string; run: PageRun }) => void;
    f.transport.open = () => new Promise((resolve) => { finish = resolve; });
    f.transport.fetch = async (ids) => { f.calls.fetch.push(ids); return { own: { run: fakeRun("fresh"), seq: f.calls.fetch.length - 1 } }; };
    const runs = createPageRuns(f.transport);
    runs.open(key("acme/x"));
    for (let i = 0; i < 1025; i++) f.emit({ runId: `foreign-${i}`, seq: 5, loaders: [], tree: true });
    finish({ runId: "own", run: fakeRun("initial") });
    await wait(COALESCE_MS + 20);
    expect(f.calls.fetch).toEqual([["own"]]);
    f.emit({ runId: "own", seq: 1, loaders: [], tree: true });
    await wait(COALESCE_MS + 20);
    expect(f.calls.fetch).toEqual([["own"], ["own"]]);
  });

  test("unknown notices with no pending opens are not retained for future opens", async () => {
    const f = fake();
    const runs = createPageRuns(f.transport);
    runs.open(key("acme/x"));
    await wait(0);
    f.emit({ runId: "run-2", seq: 10, loaders: [], tree: true });
    runs.open(key("acme/y"));
    await wait(COALESCE_MS + 20);
    expect(f.calls.fetch).toEqual([]);
  });
  test("the window roster includes any held run and notifies only when ids change", async () => {
    const f = fake();
    const runs = createPageRuns(f.transport);
    const rosters: string[][] = [];
    const off = runs.subscribeRunIds(() => rosters.push(runs.runIds()));
    const panel = runs.open(key("acme/panel"));
    const markdown = runs.open({ specYaml: "title: Inline", params: {}, streamId: null, threadId: null });
    await wait(0);
    expect(runs.runIds()).toEqual(["run-1", "run-2"]);
    expect(rosters).toEqual([["run-1"], ["run-1", "run-2"]]);
    panel.close();
    await wait(CLOSE_AFTER_MS + 20);
    expect(runs.runIds()).toEqual(["run-2"]);
    expect(rosters.at(-1)).toEqual(["run-2"]);
    f.reconnect();
    expect(rosters.at(-1)).toEqual([]);
    await wait(0);
    expect(runs.runIds()).toEqual(["run-3"]);
    off();
    markdown.close();
  });

  test("an old open response after reconnect cannot resurrect a stale run id", async () => {
    const f = fake();
    const pending: ((value: { runId: string; run: PageRun }) => void)[] = [];
    f.transport.open = () => new Promise((resolve) => pending.push(resolve));
    const runs = createPageRuns(f.transport);
    const handle = runs.open(key("acme/slow"));
    f.reconnect();
    pending[1]({ runId: "new", run: fakeRun("new") });
    await wait(0);
    pending[0]({ runId: "old", run: fakeRun("old") });
    await wait(0);
    expect(runs.runIds()).toEqual(["new"]);
    expect(handle.get().runId).toBe("new");
    expect(f.calls.close).toContain("old");
  });

  test("releasing an unresolved open closes its late result without adding it to the roster", async () => {
    const f = fake();
    let resolve!: (value: { runId: string; run: PageRun }) => void;
    f.transport.open = () => new Promise((done) => { resolve = done; });
    const runs = createPageRuns(f.transport);
    runs.open(key("acme/slow")).close();
    await wait(CLOSE_AFTER_MS + 20);
    resolve({ runId: "late", run: fakeRun("late") });
    await wait(0);
    expect(runs.runIds()).toEqual([]);
    expect(f.calls.close).toContain("late");
  });
  test("two hosts of one page share one run, closed when the last lets go", async () => {
    const f = fake();
    const runs = createPageRuns(f.transport);
    const a = runs.open(key("acme/x"));
    const b = runs.open(key("acme/x"));
    await wait(0);
    expect(f.calls.open).toBe(1);
    expect(a.get().run?.page.title).toBe("acme/x@1");
    a.close();
    await wait(CLOSE_AFTER_MS + 20);
    expect(f.calls.close).toEqual([]);
    b.close();
    await wait(CLOSE_AFTER_MS + 20);
    expect(f.calls.close).toEqual(["run-1"]);
  });

  test("changes in a burst are one fetch; an older change is dropped", async () => {
    const f = fake();
    const runs = createPageRuns(f.transport);
    const a = runs.open(key("acme/x"));
    const b = runs.open(key("acme/y"));
    await wait(0);
    let heard = 0;
    a.subscribe(() => (heard += 1));
    f.emit({ runId: "run-1", seq: 1, loaders: ["n"], tree: false });
    f.emit({ runId: "run-2", seq: 1, loaders: ["n"], tree: false });
    f.emit({ runId: "run-1", seq: 2, loaders: ["n"], tree: false });
    await wait(COALESCE_MS + 20);
    expect(f.calls.fetch).toEqual([["run-1", "run-2"]]);
    expect(a.get().run?.page.title).toBe("acme/x@fetched");
    expect(b.get().run?.page.title).toBe("acme/y@fetched");
    expect(heard).toBe(1);
    f.emit({ runId: "run-1", seq: 1, loaders: ["n"], tree: false });
    await wait(COALESCE_MS + 20);
    expect(f.calls.fetch.length).toBe(1);
  });

  test("new params re-run it, marked stale until the run answers", async () => {
    const f = fake();
    const runs = createPageRuns(f.transport);
    const a = runs.open(key("acme/x"));
    await wait(0);
    const done = a.setParams({ n: { Int: 3 } as never });
    expect(a.get().stale).toBe(true);
    await done;
    expect(a.get().stale).toBe(false);
    expect(a.get().run?.page.title).toBe("re-run");
  });

  test("a run's state names its run id once it is open", async () => {
    const f = fake();
    const runs = createPageRuns(f.transport);
    const a = runs.open(key("acme/x"));
    expect(a.get().runId).toBeNull();
    await wait(0);
    expect(a.get().runId).toBe("run-1");
  });

  test("Refresh re-runs it as it is, the old run shown until it answers", async () => {
    const f = fake();
    const runs = createPageRuns(f.transport);
    const a = runs.open(key("acme/x"));
    await wait(0);
    const done = a.rerun();
    expect(a.get().stale).toBe(true);
    expect(a.get().run?.page.title).toBe("acme/x@1");
    await done;
    expect(f.calls.rerun).toEqual(["run-1"]);
    expect(a.get().stale).toBe(false);
    expect(a.get().run?.page.title).toBe("refreshed");
  });

  test("after a reconnect every run opens again", async () => {
    const f = fake();
    const runs = createPageRuns(f.transport);
    runs.open(key("acme/x"));
    await wait(0);
    f.reconnect();
    await wait(0);
    expect(f.calls.open).toBe(2);
    expect(runs.runIds()).toEqual(["run-2"]);
  });
});

test("refresh runs an open run again by its id, for every host holding it; an unknown id does nothing", async () => {
  const { fakePageRuns, fakePage, fakeRun, node } = await import("./pages/testing/fakeRun.js");
  const { runs, calls } = fakePageRuns(fakeRun(fakePage(node.viz("items", "items"))));
  const handle = runs.open({ page: "acme/items", params: {}, streamId: null, threadId: null });
  await new Promise((r) => setTimeout(r, 0));
  expect(handle.get().runId).toBe("run-1");
  await runs.refresh("run-1");
  expect(calls.rerun).toBe(1);
  await runs.refresh("run-unknown");
  expect(calls.rerun).toBe(1);
  handle.close();
});
