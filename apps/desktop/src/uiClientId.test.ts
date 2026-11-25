import { expect, test } from "bun:test";
import { createUiClientId } from "./uiClientId.js";

// A window's id lasts one page load: its logs, tab reports, client host and
// event socket (reconnects included) share it. A reload is a new window to
// the daemon, so a late "unregister" from the page that left can never
// remove the page that took its place, and a duplicated browser tab (which
// copies session storage) is never the same window as the one it came from.
test("a window keeps one id for its whole page load", () => {
  let made = 0;
  const id = createUiClientId(() => `window-${++made}`);
  expect(id()).toBe("window-1");
  expect(id()).toBe("window-1");
  expect(made).toBe(1);
});

test("a new page load is a new window, whatever session storage holds", () => {
  const before = createUiClientId(() => "before-reload");
  const after = createUiClientId(() => "after-reload");
  expect(before()).toBe("before-reload");
  expect(after()).toBe("after-reload");
});
