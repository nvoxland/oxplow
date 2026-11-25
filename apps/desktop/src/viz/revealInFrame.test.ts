import { expect, test } from "bun:test";

import { revealInFrame } from "./revealInFrame.js";

// A component's `reveal(top, bottom)`: the host scrolls what holds the frame
// (its nearest scrolling ancestor) so that part of the frame is in view,
// moving as little as it can.

function setup(scrollTop: number) {
  const scroller = document.createElement("div");
  scroller.style.overflowY = "auto";
  Object.defineProperty(scroller, "scrollHeight", { value: 5000 });
  Object.defineProperty(scroller, "clientHeight", { value: 600 });
  scroller.scrollTop = scrollTop;
  // The scroller shows y 100–700 of the window.
  scroller.getBoundingClientRect = () => ({ top: 100, bottom: 700, left: 0, right: 800, width: 800, height: 600, x: 0, y: 100, toJSON() {} }) as DOMRect;
  const frame = document.createElement("iframe");
  scroller.appendChild(frame);
  document.body.appendChild(scroller);
  // The frame starts 300 px below the scroller's top as it is scrolled now.
  frame.getBoundingClientRect = () => ({ top: 400 - scroller.scrollTop, bottom: 4400 - scroller.scrollTop, left: 0, right: 800, width: 800, height: 4000, x: 0, y: 400, toJSON() {} }) as DOMRect;
  return { scroller, frame };
}

test("a part below the view scrolls up just enough to show its bottom", () => {
  const { scroller, frame } = setup(0);
  revealInFrame(frame, 1000, 1036);
  // The part's bottom (400 + 1036 = 1436) lands at the view's bottom (700).
  expect(scroller.scrollTop).toBe(736);
});

test("a part above the view scrolls down to show its top", () => {
  const { scroller, frame } = setup(2000);
  revealInFrame(frame, 100, 136);
  // Its top (400 + 100 − 2000 = −1500) lands at the view's top (100).
  expect(scroller.scrollTop).toBe(400);
});

test("a part already in view doesn't move it", () => {
  const { scroller, frame } = setup(0);
  revealInFrame(frame, 10, 40);
  expect(scroller.scrollTop).toBe(0);
});
