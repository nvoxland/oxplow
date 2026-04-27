import { describe, expect, test } from "bun:test";
import {
  createSlot,
  createTabStore,
  navigateInSlot,
  resetSlotCounter,
  slotGoBack,
  slotGoForward,
  type TabRef,
} from "./tabState.js";

const FILE_A: TabRef = { id: "file:src/a.ts", kind: "file", payload: { path: "src/a.ts" } };
const FILE_B: TabRef = { id: "file:src/b.ts", kind: "file", payload: { path: "src/b.ts" } };
const AGENT: TabRef = { id: "agent", kind: "agent", payload: null };
const WORK_ITEM: TabRef = { id: "wi:wi-123", kind: "work-item", payload: { itemId: "wi-123" } };

describe("tabStore", () => {
  test("a fresh thread has no tabs and no active tab", () => {
    const store = createTabStore();
    const state = store.getThreadState("t-1");
    expect(state.tabs).toEqual([]);
    expect(state.activeId).toBeNull();
  });

  test("openTab adds a tab and makes it active", () => {
    const store = createTabStore();
    store.openTab("t-1", FILE_A);
    const state = store.getThreadState("t-1");
    expect(state.tabs).toEqual([FILE_A]);
    expect(state.activeId).toBe(FILE_A.id);
  });

  test("openTab on an existing tab focuses it without duplicating", () => {
    const store = createTabStore();
    store.openTab("t-1", FILE_A);
    store.openTab("t-1", FILE_B);
    store.openTab("t-1", FILE_A);
    const state = store.getThreadState("t-1");
    expect(state.tabs).toEqual([FILE_A, FILE_B]);
    expect(state.activeId).toBe(FILE_A.id);
  });

  test("activate sets the active tab", () => {
    const store = createTabStore();
    store.openTab("t-1", FILE_A);
    store.openTab("t-1", FILE_B);
    store.activate("t-1", FILE_A.id);
    expect(store.getThreadState("t-1").activeId).toBe(FILE_A.id);
  });

  test("activate is a no-op for an unknown tab id", () => {
    const store = createTabStore();
    store.openTab("t-1", FILE_A);
    store.activate("t-1", "file:does-not-exist");
    expect(store.getThreadState("t-1").activeId).toBe(FILE_A.id);
  });

  test("closeTab removes the tab and focuses the previous one", () => {
    const store = createTabStore();
    store.openTab("t-1", AGENT);
    store.openTab("t-1", FILE_A);
    store.openTab("t-1", FILE_B);
    store.closeTab("t-1", FILE_B.id);
    const state = store.getThreadState("t-1");
    expect(state.tabs.map((t) => t.id)).toEqual([AGENT.id, FILE_A.id]);
    expect(state.activeId).toBe(FILE_A.id);
  });

  test("closing a non-active tab does not change the active tab", () => {
    const store = createTabStore();
    store.openTab("t-1", AGENT);
    store.openTab("t-1", FILE_A);
    store.openTab("t-1", FILE_B);
    store.activate("t-1", FILE_B.id);
    store.closeTab("t-1", FILE_A.id);
    const state = store.getThreadState("t-1");
    expect(state.tabs.map((t) => t.id)).toEqual([AGENT.id, FILE_B.id]);
    expect(state.activeId).toBe(FILE_B.id);
  });

  test("closing the last tab leaves activeId null", () => {
    const store = createTabStore();
    store.openTab("t-1", FILE_A);
    store.closeTab("t-1", FILE_A.id);
    const state = store.getThreadState("t-1");
    expect(state.tabs).toEqual([]);
    expect(state.activeId).toBeNull();
  });

  test("threads are isolated", () => {
    const store = createTabStore();
    store.openTab("t-1", FILE_A);
    store.openTab("t-2", WORK_ITEM);
    expect(store.getThreadState("t-1").tabs.map((t) => t.id)).toEqual([FILE_A.id]);
    expect(store.getThreadState("t-2").tabs.map((t) => t.id)).toEqual([WORK_ITEM.id]);
  });

  test("subscribe receives updates for the matching thread only", () => {
    const store = createTabStore();
    const seenT1: number[] = [];
    const seenT2: number[] = [];
    const off1 = store.subscribe("t-1", () => seenT1.push(1));
    const off2 = store.subscribe("t-2", () => seenT2.push(1));
    store.openTab("t-1", FILE_A);
    store.openTab("t-2", FILE_B);
    store.openTab("t-1", AGENT);
    expect(seenT1.length).toBe(2);
    expect(seenT2.length).toBe(1);
    off1();
    off2();
  });

  test("subscribe unsubscribe stops further notifications", () => {
    const store = createTabStore();
    let count = 0;
    const off = store.subscribe("t-1", () => {
      count++;
    });
    store.openTab("t-1", FILE_A);
    expect(count).toBe(1);
    off();
    store.openTab("t-1", FILE_B);
    expect(count).toBe(1);
  });

  test("openTab with `replace: true` reuses the active tab id rather than adding", () => {
    const store = createTabStore();
    store.openTab("t-1", FILE_A);
    store.openTab("t-1", FILE_B, { replace: true });
    const state = store.getThreadState("t-1");
    expect(state.tabs.map((t) => t.id)).toEqual([FILE_B.id]);
    expect(state.activeId).toBe(FILE_B.id);
  });

  test("ensureTab creates the tab without making it active", () => {
    const store = createTabStore();
    store.openTab("t-1", AGENT);
    store.ensureTab("t-1", FILE_A);
    const state = store.getThreadState("t-1");
    expect(state.tabs.map((t) => t.id)).toEqual([AGENT.id, FILE_A.id]);
    expect(state.activeId).toBe(AGENT.id);
  });

  test("ensureTab is a no-op for an existing tab", () => {
    const store = createTabStore();
    store.openTab("t-1", AGENT);
    store.openTab("t-1", FILE_A);
    store.ensureTab("t-1", FILE_A);
    const state = store.getThreadState("t-1");
    expect(state.tabs.map((t) => t.id)).toEqual([AGENT.id, FILE_A.id]);
    expect(state.activeId).toBe(FILE_A.id);
  });
});

describe("PageSlot navigation", () => {
  test("createSlot generates unique slot ids and seeds the current ref", () => {
    resetSlotCounter();
    const a = createSlot(FILE_A);
    const b = createSlot(FILE_B);
    expect(a.slotId).not.toBe(b.slotId);
    expect(a.current).toBe(FILE_A);
    expect(a.back).toEqual([]);
    expect(a.forward).toEqual([]);
  });

  test("navigateInSlot pushes current onto back, sets new current, clears forward", () => {
    let slot = createSlot(FILE_A);
    slot = navigateInSlot(slot, FILE_B);
    expect(slot.current).toBe(FILE_B);
    expect(slot.back).toEqual([FILE_A]);
    expect(slot.forward).toEqual([]);
    slot = navigateInSlot(slot, WORK_ITEM);
    expect(slot.current).toBe(WORK_ITEM);
    expect(slot.back).toEqual([FILE_A, FILE_B]);
  });

  test("navigateInSlot to the same ref is a no-op", () => {
    const slot = createSlot(FILE_A);
    expect(navigateInSlot(slot, FILE_A)).toBe(slot);
  });

  test("navigateInSlot to the back-stack top is treated as a back-navigation", () => {
    let slot = createSlot(FILE_A);
    slot = navigateInSlot(slot, FILE_B);
    slot = navigateInSlot(slot, FILE_A);
    expect(slot.current).toBe(FILE_A);
    expect(slot.back).toEqual([]);
    expect(slot.forward).toEqual([FILE_B]);
  });

  test("slotGoBack walks back and pushes current onto forward", () => {
    let slot = createSlot(FILE_A);
    slot = navigateInSlot(slot, FILE_B);
    const backed = slotGoBack(slot);
    expect(backed?.current).toBe(FILE_A);
    expect(backed?.back).toEqual([]);
    expect(backed?.forward).toEqual([FILE_B]);
  });

  test("slotGoBack returns null when back is empty", () => {
    expect(slotGoBack(createSlot(FILE_A))).toBeNull();
  });

  test("slotGoForward walks forward after a back", () => {
    let slot = createSlot(FILE_A);
    slot = navigateInSlot(slot, FILE_B);
    slot = slotGoBack(slot)!;
    const fwd = slotGoForward(slot);
    expect(fwd?.current).toBe(FILE_B);
    expect(fwd?.back).toEqual([FILE_A]);
    expect(fwd?.forward).toEqual([]);
  });

  test("slotGoForward returns null when forward is empty", () => {
    expect(slotGoForward(createSlot(FILE_A))).toBeNull();
  });

  test("a fresh navigateInSlot after going back clears forward", () => {
    let slot = createSlot(FILE_A);
    slot = navigateInSlot(slot, FILE_B);
    slot = slotGoBack(slot)!;
    slot = navigateInSlot(slot, WORK_ITEM);
    expect(slot.forward).toEqual([]);
    expect(slot.back).toEqual([FILE_A]);
    expect(slot.current).toBe(WORK_ITEM);
  });

  test("slotId is preserved across navigations", () => {
    let slot = createSlot(FILE_A, "slot:fixed");
    slot = navigateInSlot(slot, FILE_B);
    slot = slotGoBack(slot)!;
    slot = slotGoForward(slot)!;
    expect(slot.slotId).toBe("slot:fixed");
  });
});
