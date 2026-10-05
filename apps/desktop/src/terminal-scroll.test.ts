import { describe, expect, test } from "bun:test";
import { shouldHandleTerminalPageKey } from "./terminal-scroll.js";

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
