import { describe, expect, test } from "bun:test";
import {
  shouldHandleTerminalPageKey,
  shouldReturnTerminalToPrompt,
  shouldRouteWheelToTmuxHistory,
  wheelDeltaToScrollLines,
} from "./terminal-scroll.js";

describe("shouldHandleTerminalPageKey", () => {
  test("handles plain page navigation locally", () => {
    expect(keyLike("PageUp")).toBe(true);
    expect(keyLike("PageDown")).toBe(true);
  });

  test("does not intercept modified keys", () => {
    expect(keyLike("PageUp", { shiftKey: true })).toBe(false);
    expect(keyLike("PageDown", { ctrlKey: true })).toBe(false);
    expect(keyLike("PageUp", { metaKey: true })).toBe(false);
  });
});

describe("wheelDeltaToScrollLines", () => {
  test("maps pixel wheel deltas to local line scrolls", () => {
    expect(wheelDeltaToScrollLines({ deltaY: 32, deltaMode: 0 })).toBe(4);
    expect(wheelDeltaToScrollLines({ deltaY: -40, deltaMode: 0 })).toBe(-5);
  });

  test("maps line and page delta modes", () => {
    expect(wheelDeltaToScrollLines({ deltaY: 3, deltaMode: 1 })).toBe(3);
    expect(wheelDeltaToScrollLines({ deltaY: -1, deltaMode: 2 })).toBe(-12);
  });

  test("preserves tiny wheel movement direction", () => {
    expect(wheelDeltaToScrollLines({ deltaY: 0.5, deltaMode: 0 })).toBe(1);
    expect(wheelDeltaToScrollLines({ deltaY: -0.5, deltaMode: 0 })).toBe(-1);
  });
});

describe("shouldReturnTerminalToPrompt", () => {
  test("treats prompt-oriented keys as a return to live input", () => {
    expect(promptKey("a")).toBe(true);
    expect(promptKey("Enter")).toBe(true);
    expect(promptKey("Backspace")).toBe(true);
    expect(promptKey("Escape")).toBe(true);
  });

  test("does not intercept modified or navigation keys", () => {
    expect(promptKey("ArrowUp")).toBe(false);
    expect(promptKey("PageUp")).toBe(false);
    expect(promptKey("a", { ctrlKey: true })).toBe(false);
    expect(promptKey("x", { metaKey: true })).toBe(false);
  });
});

describe("shouldRouteWheelToTmuxHistory", () => {
  test("keeps wheel in tmux history while already in history mode", () => {
    expect(shouldRouteWheelToTmuxHistory({
      mode: "history",
      bufferType: "normal",
      mouseTrackingMode: "any",
    })).toBe(true);
  });

  test("routes alternate screen wheel to tmux when mouse tracking is off", () => {
    expect(shouldRouteWheelToTmuxHistory({
      mode: "live",
      bufferType: "alternate",
      mouseTrackingMode: "none",
    })).toBe(true);
  });

  test("lets xterm handle wheel when mouse tracking is active", () => {
    expect(shouldRouteWheelToTmuxHistory({
      mode: "live",
      bufferType: "alternate",
      mouseTrackingMode: "vt200",
    })).toBe(false);
  });

  test("lets xterm handle wheel in the normal buffer", () => {
    expect(shouldRouteWheelToTmuxHistory({
      mode: "live",
      bufferType: "normal",
      mouseTrackingMode: "none",
    })).toBe(false);
  });
});

function keyLike(
  key: string,
  overrides: Partial<{
    altKey: boolean;
    ctrlKey: boolean;
    metaKey: boolean;
    shiftKey: boolean;
  }> = {},
) {
  return shouldHandleTerminalPageKey({
    key,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    ...overrides,
  });
}

function promptKey(
  key: string,
  overrides: Partial<{
    altKey: boolean;
    ctrlKey: boolean;
    metaKey: boolean;
  }> = {},
) {
  return shouldReturnTerminalToPrompt({
    key,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    ...overrides,
  });
}
