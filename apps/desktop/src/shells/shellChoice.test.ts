import { expect, test } from "bun:test";

import { chooseShell, shellOverride } from "./shellChoice.js";

// Which shell draws the window: a test's override, else the device's own
// choice, else its kind's default (a tablet or phone gets the tablet shell
// until a phone shell exists).
test("the override wins, then the device's choice, then its kind", () => {
  expect(chooseShell({ override: "tablet", preference: "desktop", deviceKind: "desktop" })).toBe("tablet");
  expect(chooseShell({ override: null, preference: "desktop", deviceKind: "tablet" })).toBe("desktop");
  expect(chooseShell({ override: null, preference: null, deviceKind: "tablet" })).toBe("tablet");
  expect(chooseShell({ override: null, preference: null, deviceKind: "phone" })).toBe("tablet");
  expect(chooseShell({ override: null, preference: null, deviceKind: "desktop" })).toBe("desktop");
});

test("an unknown value is no choice", () => {
  expect(chooseShell({ override: null, preference: "wobbly", deviceKind: "desktop" })).toBe("desktop");
});

test("the override is the page's ?shell= or the window context's shell", () => {
  expect(shellOverride("?shell=tablet", null)).toBe("tablet");
  expect(shellOverride("", { shell: "desktop" })).toBe("desktop");
  expect(shellOverride("?shell=nope", null)).toBeNull();
  expect(shellOverride("", null)).toBeNull();
});
