import { describe, expect, test } from "bun:test";

import {
  MENU_MARGIN,
  clampMenuPosition,
  submenuTopOffset,
} from "./contextMenuPosition.js";

const VIEWPORT = { width: 1200, height: 800 };

describe("clampMenuPosition", () => {
  test("leaves a menu that already fits where it was opened", () => {
    const at = clampMenuPosition({ x: 300, y: 200 }, { width: 220, height: 150 }, VIEWPORT);
    expect(at).toEqual({ x: 300, y: 200 });
  });

  test("pulls a menu back from the right and bottom edges", () => {
    const at = clampMenuPosition({ x: 1150, y: 780 }, { width: 220, height: 150 }, VIEWPORT);
    expect(at.x).toBe(1200 - 220 - MENU_MARGIN);
    expect(at.y).toBe(800 - 150 - MENU_MARGIN);
  });

  test("never places a menu above or left of the margin", () => {
    const at = clampMenuPosition({ x: -50, y: -50 }, { width: 220, height: 150 }, VIEWPORT);
    expect(at).toEqual({ x: MENU_MARGIN, y: MENU_MARGIN });
  });

  test("a menu taller than the viewport pins to the top rather than hanging off it", () => {
    // tsk146: the whole point. Before the max-height existed, a menu this tall
    // pinned here and everything past the fold was simply unreachable — there
    // was no scroll. Position alone can't fix that; it's why the fix is
    // max-height + overflowY, and this only asserts the position half.
    const at = clampMenuPosition({ x: 400, y: 400 }, { width: 220, height: 2000 }, VIEWPORT);
    expect(at.y).toBe(MENU_MARGIN);
    expect(at.y).toBeGreaterThanOrEqual(0);
  });
});

describe("submenuTopOffset", () => {
  test("a submenu that fits is not moved", () => {
    expect(submenuTopOffset({ top: 100, height: 200 }, 800)).toBe(0);
  });

  test("a submenu overhanging the bottom is lifted exactly enough to fit", () => {
    // bottom = 700 + 200 = 900; the limit is 800 - 8 = 792 → lift by 108.
    expect(submenuTopOffset({ top: 700, height: 200 }, 800)).toBe(-108);
  });

  test("a submenu taller than the viewport is NOT lifted off the top", () => {
    // The reported bug: ~30 items opening near the bottom. Lifting by the full
    // overflow would put the top at 600 - 1192 = -592, so the first entries sit
    // above the window and cannot be reached or scrolled to.
    const offset = submenuTopOffset({ top: 600, height: 1400 }, 800);
    const resultingTop = 600 + offset;
    expect(resultingTop).toBe(MENU_MARGIN);
    expect(resultingTop).toBeGreaterThanOrEqual(0);
  });

  test("the top stays on screen for any height, at any anchor", () => {
    // Property sweep rather than a single case — the failure was an unbounded
    // shift, so the invariant worth guarding is "top never leaves the screen".
    for (const top of [0, 50, 300, 600, 799]) {
      for (const height of [10, 400, 800, 1500, 5000]) {
        const resultingTop = top + submenuTopOffset({ top, height }, 800);
        expect(resultingTop).toBeGreaterThanOrEqual(0);
      }
    }
  });
});
