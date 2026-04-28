import { describe, expect, test } from "bun:test";
import { createOpErrorsStore } from "./opErrorsStore.js";

describe("opErrorsStore", () => {
  test("push prepends and notifies listeners", () => {
    const store = createOpErrorsStore();
    let calls = 0;
    store.subscribe(() => { calls++; });
    const id1 = store.push({ label: "first", stderr: "boom" });
    const id2 = store.push({ label: "second", stderr: "kaboom" });
    expect(calls).toBe(2);
    const snap = store.getSnapshot();
    expect(snap[0]?.id).toBe(id2);
    expect(snap[1]?.id).toBe(id1);
    expect(snap[0]?.label).toBe("second");
    expect(snap[0]?.seen).toBe(false);
  });

  test("markSeen flips seen flag once", () => {
    const store = createOpErrorsStore();
    const id = store.push({ label: "x" });
    let calls = 0;
    store.subscribe(() => { calls++; });
    store.markSeen(id);
    expect(store.getSnapshot()[0]?.seen).toBe(true);
    store.markSeen(id);
    expect(calls).toBe(1);
  });

  test("dismiss removes only the matching entry", () => {
    const store = createOpErrorsStore();
    const a = store.push({ label: "a" });
    store.push({ label: "b" });
    store.dismiss(a);
    const snap = store.getSnapshot();
    expect(snap.length).toBe(1);
    expect(snap[0]?.label).toBe("b");
  });

  test("clear empties the store", () => {
    const store = createOpErrorsStore();
    store.push({ label: "a" });
    store.push({ label: "b" });
    store.clear();
    expect(store.getSnapshot()).toEqual([]);
  });

  test("caps at MAX_ENTRIES (20) keeping newest", () => {
    const store = createOpErrorsStore();
    for (let i = 0; i < 25; i++) store.push({ label: `e${i}` });
    const snap = store.getSnapshot();
    expect(snap.length).toBe(20);
    expect(snap[0]?.label).toBe("e24");
    expect(snap[19]?.label).toBe("e5");
  });

  test("push tags entry with active thread when threadId not provided", () => {
    const store = createOpErrorsStore();
    store.setActiveThread("b-thread-1");
    store.push({ label: "a" });
    store.setActiveThread("b-thread-2");
    store.push({ label: "b" });
    store.setActiveThread(null);
    store.push({ label: "c" });
    const [c, b, a] = store.getSnapshot();
    expect(a?.threadId).toBe("b-thread-1");
    expect(b?.threadId).toBe("b-thread-2");
    expect(c?.threadId).toBe(null);
  });

  test("explicit threadId in push input overrides active thread", () => {
    const store = createOpErrorsStore();
    store.setActiveThread("b-active");
    store.push({ label: "a", threadId: "b-explicit" });
    store.push({ label: "b", threadId: null });
    const [b, a] = store.getSnapshot();
    expect(a?.threadId).toBe("b-explicit");
    expect(b?.threadId).toBe(null);
  });
});
