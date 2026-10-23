import { expect, test } from "bun:test";

import { createBufferStore, isDirty, type BufferIO, type ModelHandle, type StreamServices } from "./buffers.js";
import type { Stream } from "../api.js";

// The working tree's open files: one buffer per (stream, path), its saved
// text and its draft, held for as long as a tab shows the file. A Monaco
// model per buffer holds the draft; edits to the model (typing, an LSP
// edit) are the draft.

const stream = { id: "str1", worktree_path: "/w" } as Stream;

class FakeModel implements ModelHandle {
  listeners: (() => void)[] = [];
  disposed = false;
  constructor(public text: string) {}
  getValue() {
    return this.text;
  }
  setValue(text: string) {
    this.text = text;
    for (const l of this.listeners) l();
  }
  onDidChangeContent(cb: () => void) {
    this.listeners.push(cb);
    return { dispose: () => (this.listeners = this.listeners.filter((l) => l !== cb)) };
  }
  dispose() {
    this.disposed = true;
  }
  /** Typing (or an LSP edit) into the model. */
  type(text: string) {
    this.setValue(text);
  }
}

function fixture(files: Record<string, string | null> = { "a.rs": "fn a() {}" }) {
  const disk = { ...files };
  const saves: { path: string; content: string; call: unknown }[] = [];
  const models = new Map<string, FakeModel>();
  const services: string[] = [];
  const io: BufferIO = {
    read: async (_s, path) => disk[path] ?? null,
    save: async (_s, path, content, call) => {
      saves.push({ path, content, call });
      disk[path] = content;
    },
    model: async (_s, path, text) => {
      const m = new FakeModel(text);
      models.set(path, m);
      return m;
    },
    services: (s): StreamServices => {
      services.push(`start ${s.id}`);
      return {
        opened: (b) => services.push(`open ${b.path}`),
        changed: (b) => services.push(`change ${b.path}`),
        saved: (b) => services.push(`save ${b.path}`),
        closed: (path) => services.push(`close ${path}`),
        dispose: () => services.push(`stop ${s.id}`),
      };
    },
  };
  return { store: createBufferStore(io), disk, saves, models, services };
}

test("opening reads the file once; the buffer is clean and its model holds the text", async () => {
  const { store, models } = fixture();
  const b = await store.open(stream, "a.rs");
  expect(b.state).toBe("ready");
  expect(b.saved).toBe("fn a() {}");
  expect(isDirty(b)).toBe(false);
  expect(models.get("a.rs")!.getValue()).toBe("fn a() {}");
  await store.open(stream, "a.rs");
  expect(models.size).toBe(1);
});

test("a file that isn't there opens as missing, with no model", async () => {
  const { store, models } = fixture({});
  const b = await store.open(stream, "gone.rs");
  expect(b.state).toBe("missing");
  expect(models.size).toBe(0);
});

test("typing into the model is the draft; saving writes it and the buffer is clean again", async () => {
  const { store, models, saves } = fixture();
  await store.open(stream, "a.rs");
  models.get("a.rs")!.type("fn b() {}");
  expect(isDirty(store.get("str1", "a.rs")!)).toBe(true);
  expect(store.get("str1", "a.rs")!.draft).toBe("fn b() {}");
  await store.save("str1", "a.rs");
  expect(saves).toEqual([{ path: "a.rs", content: "fn b() {}", call: null }]);
  expect(isDirty(store.get("str1", "a.rs")!)).toBe(false);
});

test("an agent's save is written for its call", async () => {
  const { store, models, saves } = fixture();
  await store.open(stream, "a.rs");
  models.get("a.rs")!.type("x");
  const call = { client: "c", id: "1" };
  await store.save("str1", "a.rs", call);
  expect(saves[0]!.call).toEqual(call);
});

test("subscribers hear each change", async () => {
  const { store, models } = fixture();
  let heard = 0;
  const off = store.subscribe(() => heard++);
  await store.open(stream, "a.rs");
  const before = heard;
  models.get("a.rs")!.type("y");
  expect(heard).toBeGreaterThan(before);
  off();
});

test("buffers follow the tabs: one no tab holds is closed, its model disposed; a dirty one can be stashed and restored", async () => {
  const { store, models } = fixture({ "a.rs": "A", "b.rs": "B" });
  await store.open(stream, "a.rs");
  await store.open(stream, "b.rs");
  models.get("a.rs")!.type("A2");
  const stash = store.stash("str1", "a.rs");
  expect(stash).toEqual({ stream, path: "a.rs", saved: "A", draft: "A2" });
  store.retainOnly([{ stream: "str1", path: "b.rs" }]);
  expect(store.get("str1", "a.rs")).toBeUndefined();
  expect(models.get("a.rs")!.disposed).toBe(true);
  await store.restore(stash!);
  expect(store.get("str1", "a.rs")!.draft).toBe("A2");
  expect(isDirty(store.get("str1", "a.rs")!)).toBe(true);
});

test("a clean buffer has nothing to stash", async () => {
  const { store } = fixture();
  await store.open(stream, "a.rs");
  expect(store.stash("str1", "a.rs")).toBeNull();
});

test("a rename moves the buffers under it; a delete closes them", async () => {
  const { store } = fixture({ "src/a.rs": "A", "src/b.rs": "B", "c.rs": "C" });
  await store.open(stream, "src/a.rs");
  await store.open(stream, "src/b.rs");
  await store.open(stream, "c.rs");
  store.renamed("str1", "src", "lib");
  expect(store.get("str1", "lib/a.rs")?.saved).toBe("A");
  expect(store.get("str1", "src/a.rs")).toBeUndefined();
  store.deleted("str1", "lib/b.rs");
  expect(store.get("str1", "lib/b.rs")).toBeUndefined();
  expect(store.get("str1", "c.rs")).toBeTruthy();
});

test("the disk changing under a clean buffer replaces it; under a draft it asks, and Reload or Keep Mine decides", async () => {
  const { store, models, disk } = fixture({ "a.rs": "A" });
  await store.open(stream, "a.rs");
  disk["a.rs"] = "A from disk";
  await store.diskChanged("str1", "a.rs");
  expect(store.get("str1", "a.rs")!.draft).toBe("A from disk");
  expect(models.get("a.rs")!.getValue()).toBe("A from disk");

  models.get("a.rs")!.type("mine");
  disk["a.rs"] = "theirs";
  await store.diskChanged("str1", "a.rs");
  expect(store.get("str1", "a.rs")!.conflict).toBe("theirs");
  store.keepMine("str1", "a.rs");
  expect(store.get("str1", "a.rs")!.conflict).toBeNull();
  expect(store.get("str1", "a.rs")!.saved).toBe("theirs");
  expect(store.get("str1", "a.rs")!.draft).toBe("mine");

  disk["a.rs"] = "theirs again";
  await store.diskChanged("str1", "a.rs");
  store.reload("str1", "a.rs");
  expect(store.get("str1", "a.rs")!.draft).toBe("theirs again");
  expect(isDirty(store.get("str1", "a.rs")!)).toBe(false);
});

test("a stream's language services start with its first buffer and stop with its last, told of each open, edit, save and close", async () => {
  const { store, models, services } = fixture({ "a.rs": "A", "b.rs": "B" });
  await store.open(stream, "a.rs");
  await store.open(stream, "b.rs");
  models.get("a.rs")!.type("A2");
  await store.save("str1", "a.rs");
  store.retainOnly([{ stream: "str1", path: "a.rs" }]);
  store.retainOnly([]);
  expect(services).toEqual([
    "start str1",
    "open a.rs",
    "open b.rs",
    "change a.rs",
    "save a.rs",
    "close b.rs",
    "close a.rs",
    "stop str1",
  ]);
});

test("the dirty files of a stream are what an agent is told is open", async () => {
  const { store, models } = fixture({ "a.rs": "A", "b.rs": "B" });
  await store.open(stream, "a.rs");
  await store.open(stream, "b.rs");
  models.get("b.rs")!.type("B2");
  expect(store.list("str1").map((b) => [b.path, isDirty(b)])).toEqual([
    ["a.rs", false],
    ["b.rs", true],
  ]);
});
