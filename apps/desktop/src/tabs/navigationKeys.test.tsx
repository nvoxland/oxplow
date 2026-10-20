import { expect, test } from "bun:test";

import { navigationKey } from "./navigationKeys.js";

const press = (key: string, mods: Partial<{ metaKey: boolean; altKey: boolean; ctrlKey: boolean; shiftKey: boolean }> = {}) => ({
  key,
  metaKey: false,
  altKey: false,
  ctrlKey: false,
  shiftKey: false,
  ...mods,
});

test("Cmd+[ and Cmd+] are Back and Forward; Alt+Up and Alt+Down step the origin", () => {
  expect(navigationKey(press("[", { metaKey: true }), document.body)).toBe("back");
  expect(navigationKey(press("]", { metaKey: true }), document.body)).toBe("forward");
  expect(navigationKey(press("ArrowUp", { altKey: true }), document.body)).toBe("up");
  expect(navigationKey(press("ArrowDown", { altKey: true }), document.body)).toBe("down");
});

test("other chords and plain keys are not navigation", () => {
  expect(navigationKey(press("["), document.body)).toBeNull();
  expect(navigationKey(press("[", { metaKey: true, shiftKey: true }), document.body)).toBeNull();
  expect(navigationKey(press("ArrowUp"), document.body)).toBeNull();
  expect(navigationKey(press("ArrowUp", { altKey: true, metaKey: true }), document.body)).toBeNull();
});

test("typing in a field, a select, a contenteditable or Monaco keeps the keys", () => {
  for (const tag of ["input", "textarea", "select"]) {
    const el = document.createElement(tag);
    document.body.append(el);
    expect(navigationKey(press("[", { metaKey: true }), el)).toBeNull();
    el.remove();
  }
  const editable = document.createElement("div");
  editable.contentEditable = "true";
  document.body.append(editable);
  expect(navigationKey(press("ArrowDown", { altKey: true }), editable)).toBeNull();
  editable.remove();
  const monaco = document.createElement("div");
  monaco.className = "monaco-editor";
  const inner = document.createElement("div");
  monaco.append(inner);
  document.body.append(monaco);
  expect(navigationKey(press("]", { metaKey: true }), inner)).toBeNull();
  monaco.remove();
});
