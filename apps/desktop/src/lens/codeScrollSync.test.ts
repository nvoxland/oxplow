import { expect, test } from "bun:test";

import { ScrollSync, type Scrollable } from "./codeScrollSync.js";

// Editors that scroll together line their marks up, then move by the
// same distance from there — never to the same absolute position.

function editor(sync: ScrollSync, key: string): Scrollable & { top: number; left: number } {
  const e = {
    top: 0,
    left: 0,
    setScrollTop(top: number) {
      e.top = top;
      // Monaco reports the move it was told to make.
      sync.scrolled(key, top, e.left);
    },
    setScrollLeft(left: number) {
      e.left = left;
    },
  };
  return e;
}

test("the marks line up two lines below the top, or less when one is near its file's top", () => {
  const sync = new ScrollSync();
  const a = editor(sync, "a");
  const b = editor(sync, "b");
  sync.register("a", a);
  sync.register("b", b);
  sync.place("a", 1000, 20);
  expect(a.top).toBe(960);
  sync.place("b", 10, 20);
  // b's mark is 10px down: both pad by 10 so the marks sit level.
  expect([a.top, b.top]).toEqual([990, 0]);
});

test("a scroll moves the others the same distance from their anchors", () => {
  const sync = new ScrollSync();
  const a = editor(sync, "a");
  const b = editor(sync, "b");
  sync.register("a", a);
  sync.register("b", b);
  sync.place("a", 1000, 20);
  sync.place("b", 400, 20);
  expect([a.top, b.top]).toEqual([960, 360]);
  sync.scrolled("a", 1060, 30);
  expect([b.top, b.left]).toEqual([460, 30]);
  sync.scrolled("a", 0, 0);
  expect(b.top).toBe(0);
});

test("a removed editor is no longer moved", () => {
  const sync = new ScrollSync();
  const a = editor(sync, "a");
  const b = editor(sync, "b");
  sync.register("a", a);
  const off = sync.register("b", b);
  sync.place("a", 1000, 20);
  sync.place("b", 400, 20);
  off();
  sync.scrolled("a", 1100, 0);
  expect(b.top).toBe(360);
});
