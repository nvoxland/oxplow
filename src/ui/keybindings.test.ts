import { describe, expect, test } from "bun:test";
import { getCommandIdForShortcut } from "./keybindings.js";

describe("getCommandIdForShortcut", () => {
  test("maps ctrl/cmd shortcuts to command ids", () => {
    expect(getCommandIdForShortcut(eventLike("s", { metaKey: true }))).toBe("file.save");
    expect(getCommandIdForShortcut(eventLike("p", { ctrlKey: true }))).toBe("file.quickOpen");
    expect(getCommandIdForShortcut(eventLike("f", { metaKey: true }))).toBe("edit.find");
  });

  test("ignores non-command key presses", () => {
    expect(getCommandIdForShortcut(eventLike("s"))).toBeNull();
    expect(getCommandIdForShortcut(eventLike("f", { altKey: true, metaKey: true }))).toBeNull();
    expect(getCommandIdForShortcut(eventLike("x", { ctrlKey: true }))).toBeNull();
  });
});

function eventLike(
  key: string,
  overrides: Partial<{
    metaKey: boolean;
    ctrlKey: boolean;
    shiftKey: boolean;
    altKey: boolean;
  }> = {},
) {
  return {
    key,
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    ...overrides,
  };
}
