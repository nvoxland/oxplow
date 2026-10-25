import { describe, expect, test } from "bun:test";

import { CLOSE_AFTER_MS, COALESCE_MS, createPageRuns, type PageRunsTransport, type RunChange } from "./pageRuns.js";
import type { PageRun } from "./tauri-bridge/generated/bindings.js";

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

function fakeRun(label: string): PageRun {
  return { page: { title: label } } as unknown as PageRun;
}

function fake() {
  let next = 0;
  const calls = { open: 0, close: [] as string[], fetch: [] as string[][], setParams: 0, rerun: [] as string[] };
  let change: (c: RunChange) => void = () => {};
  let reconnect: () => void = () => {};
  const current = new Map<string, string>();
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
      return Object.fromEntries(ids.map((id) => [id, fakeRun(`${current.get(id)}@fetched`)]));
    },
    setParams: async () => {
      calls.setParams += 1;
      return fakeRun("re-run");
    },
    rerun: async (runId) => {
      calls.rerun.push(runId);
      return fakeRun("refreshed");
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
  return { transport, calls, emit: (c: RunChange) => change(c), reconnect: () => reconnect() };
}

const key = (page: string) => ({ page, params: {}, streamId: null, threadId: null });

describe("pageRuns", () => {
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
