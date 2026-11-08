import { expect, test } from "bun:test";

import { createFailureStore, createInlineClaims } from "./actionFailures.js";

test("a failure stays with its anchor's key while the control remounts", () => {
  const store = createFailureStore();
  let heard = 0;
  store.subscribe(() => (heard += 1));
  const unmount = store.mount("proposal/8");
  store.set("proposal/8", { message: "no", code: "FAILED", errorId: null, at: 1 });
  expect(heard).toBe(1);
  // A re-read drops the card and draws it again: the key is mounted twice
  // for a moment, then once; the failure is still there.
  const again = store.mount("proposal/8");
  unmount();
  expect(store.isMounted("proposal/8")).toBe(true);
  expect(store.get("proposal/8")?.message).toBe("no");
  again();
  expect(store.isMounted("proposal/8")).toBe(false);
  store.clear("proposal/8");
  expect(store.get("proposal/8")).toBeNull();
});

/** A clock the test moves. */
function clock() {
  const timers: { at: number; f: () => void }[] = [];
  let now = 0;
  return {
    later: (f: () => void, ms: number) => void timers.push({ at: now + ms, f }),
    advance: async (ms: number) => {
      now += ms;
      for (const t of timers.filter((t) => t.at <= now)) {
        timers.splice(timers.indexOf(t), 1);
        t.f();
      }
      await new Promise((r) => setTimeout(r, 0));
    },
  };
}

test("a claimed report is shown inline when its control is still on screen after the grace", async () => {
  const c = clock();
  const claims = createInlineClaims(c.later, 750);
  let resolve!: (id: string | null) => void;
  claims.claim(new Promise((r) => (resolve = r)), () => true);
  let settled = false;
  void claims.settled().then(() => (settled = true));
  resolve("ev1");
  await c.advance(0);
  // The toast waits: the grace hasn't passed.
  expect(settled).toBe(false);
  await c.advance(750);
  expect(settled).toBe(true);
  expect(claims.shown("ev1")).toBe(true);
});

test("a claimed report whose control left is the toast's", async () => {
  const c = clock();
  const claims = createInlineClaims(c.later, 750);
  claims.claim(Promise.resolve("ev2"), () => false);
  await c.advance(750);
  await claims.settled();
  expect(claims.shown("ev2")).toBe(false);
});

test("settled resolves at once with nothing claimed, and a failed report claims nothing", async () => {
  const c = clock();
  const claims = createInlineClaims(c.later, 750);
  await claims.settled();
  claims.claim(Promise.resolve(null), () => true);
  await c.advance(750);
  await claims.settled();
});
