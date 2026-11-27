import { expect, test } from "bun:test";

import { touchScrollStep, wheelAction } from "./touchScroll.js";

// A finger's drag scrolls whole lines: the movement left over (less than a
// line) waits for the next move. A finger moving down shows the lines above
// (a wheel up, negative).
test("a drag of whole lines scrolls them; the rest waits", () => {
  expect(touchScrollStep(0, 40, 17)).toEqual({ lines: -2, rest: 6 });
  expect(touchScrollStep(6, 12, 17)).toEqual({ lines: -1, rest: 1 });
  expect(touchScrollStep(0, -35, 17)).toEqual({ lines: 2, rest: -1 });
  expect(touchScrollStep(0, 10, 17)).toEqual({ lines: 0, rest: 10 });
  expect(touchScrollStep(0, 10, 0)).toEqual({ lines: 0, rest: 0 });
});

// What a drag of `lines` does, as a wheel would in each mode.
test("a shell scrolls; an alternate screen gets arrows; a program with the mouse gets wheel reports", () => {
  const at = { col: 3, row: 5 };
  expect(wheelAction({ mouse: false, alternate: false, applicationCursor: false }, -2, at)).toEqual({ scroll: -2 });
  expect(wheelAction({ mouse: false, alternate: true, applicationCursor: true }, -2, at)).toEqual({ send: "\x1bOA\x1bOA" });
  expect(wheelAction({ mouse: false, alternate: true, applicationCursor: false }, 1, at)).toEqual({ send: "\x1b[B" });
  // SGR: button 64 is a wheel up, 65 down, at the 1-based cell.
  expect(wheelAction({ mouse: true, alternate: false, applicationCursor: false }, -2, at)).toEqual({ send: "\x1b[<64;4;6M\x1b[<64;4;6M" });
  expect(wheelAction({ mouse: true, alternate: true, applicationCursor: false }, 1, at)).toEqual({ send: "\x1b[<65;4;6M" });
});
