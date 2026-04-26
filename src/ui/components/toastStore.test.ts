import { describe, expect, test } from "bun:test";
import { createToastStore } from "./toastStore.js";

describe("createToastStore", () => {
  test("starts empty", () => {
    const store = createToastStore();
    expect(store.getSnapshot()).toEqual([]);
  });

  test("push adds toast and returns id", () => {
    const store = createToastStore();
    const id = store.push({ message: "Deleted x" });
    const list = store.getSnapshot();
    expect(list.length).toBe(1);
    expect(list[0]?.id).toBe(id);
    expect(list[0]?.message).toBe("Deleted x");
  });

  test("dismiss removes toast", () => {
    const store = createToastStore();
    const id = store.push({ message: "x" });
    store.dismiss(id);
    expect(store.getSnapshot()).toEqual([]);
  });

  test("undo runs callback once and dismisses", () => {
    const store = createToastStore();
    let count = 0;
    const id = store.push({ message: "x", onUndo: () => { count += 1; } });
    store.undo(id);
    expect(count).toBe(1);
    expect(store.getSnapshot()).toEqual([]);
    // double-fire safety
    store.undo(id);
    expect(count).toBe(1);
  });

  test("subscribe fires on push/dismiss/undo", () => {
    const store = createToastStore();
    let fires = 0;
    const unsub = store.subscribe(() => { fires += 1; });
    const id = store.push({ message: "x" });
    expect(fires).toBe(1);
    store.dismiss(id);
    expect(fires).toBe(2);
    unsub();
    store.push({ message: "y" });
    expect(fires).toBe(2);
  });

  test("multiple toasts maintain insertion order", () => {
    const store = createToastStore();
    const a = store.push({ message: "a" });
    const b = store.push({ message: "b" });
    const c = store.push({ message: "c" });
    expect(store.getSnapshot().map((t) => t.id)).toEqual([a, b, c]);
  });

  test("toast without onUndo: undo no-ops but dismisses", () => {
    const store = createToastStore();
    const id = store.push({ message: "x" });
    store.undo(id);
    expect(store.getSnapshot()).toEqual([]);
  });
});
