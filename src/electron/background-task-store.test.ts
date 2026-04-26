import { describe, expect, it } from "bun:test";
import { BackgroundTaskStore } from "./background-task-store.js";

function sleep(ms: number) {
  return new Promise<void>((resolve) => setTimeout(resolve, ms));
}

describe("BackgroundTaskStore", () => {
  it("starts a task and lists it as running", () => {
    const store = new BackgroundTaskStore();
    const id = store.start({ kind: "git", label: "Pulling main" });
    const list = store.list();
    expect(list).toHaveLength(1);
    expect(list[0].id).toBe(id);
    expect(list[0].status).toBe("running");
    expect(list[0].label).toBe("Pulling main");
    expect(list[0].progress).toBeNull();
    store.dispose();
  });

  it("emits started/updated/ended events", () => {
    const store = new BackgroundTaskStore(undefined, 50);
    const events: string[] = [];
    store.subscribe((change) => events.push(change.kind));
    const id = store.start({ kind: "lsp", label: "boot" });
    store.update(id, { progress: 0.5 });
    store.complete(id);
    expect(events).toEqual(["started", "updated", "ended"]);
    store.dispose();
  });

  it("clamps progress and ignores updates after completion", () => {
    const store = new BackgroundTaskStore(undefined, 50);
    const id = store.start({ kind: "notes-resync", label: "sync", progress: 2 });
    expect(store.list()[0].progress).toBe(1);
    store.update(id, { progress: -0.5 });
    expect(store.list()[0].progress).toBe(0);
    store.complete(id);
    store.update(id, { progress: 0.2 });
    // post-complete update is a no-op; status remains done with progress=1
    expect(store.list()[0].status).toBe("done");
    expect(store.list()[0].progress).toBe(1);
    store.dispose();
  });

  it("records failure with an error message", () => {
    const store = new BackgroundTaskStore(undefined, 50);
    const id = store.start({ kind: "git", label: "Fetching" });
    store.fail(id, "boom");
    const row = store.list()[0];
    expect(row.status).toBe("failed");
    expect(row.error).toBe("boom");
    store.dispose();
  });

  it("auto-evicts done tasks after the grace window", async () => {
    const store = new BackgroundTaskStore(undefined, 50);
    const id = store.start({ kind: "code-quality", label: "scan" });
    store.complete(id);
    expect(store.list()).toHaveLength(1);
    await sleep(150);
    expect(store.list()).toHaveLength(0);
    store.dispose();
  });

  it("auto-evicts failed tasks after the grace window", async () => {
    const store = new BackgroundTaskStore(undefined, 50);
    const id = store.start({ kind: "git", label: "push" });
    store.fail(id, "denied");
    expect(store.list()).toHaveLength(1);
    await sleep(150);
    expect(store.list()).toHaveLength(0);
    store.dispose();
  });

  it("orders running tasks before completed ones", () => {
    const store = new BackgroundTaskStore(undefined, 50);
    const a = store.start({ kind: "git", label: "a" });
    const b = store.start({ kind: "lsp", label: "b" });
    store.complete(a);
    const list = store.list();
    expect(list[0].id).toBe(b);
    expect(list[1].id).toBe(a);
    store.dispose();
  });
});
