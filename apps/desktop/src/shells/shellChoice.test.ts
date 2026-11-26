import { expect, test } from "bun:test";

import { chooseShell, layoutItems, shellComplete, shellOverride } from "./shellChoice.js";

// Which shell draws the window: a test's override, else the device's own
// choice, else its kind's default (a tablet or phone gets the tablet shell
// until a phone shell exists).
test("the override wins, then the device's choice, then its kind", () => {
  const complete = () => true;
  expect(chooseShell({ override: "tablet", preference: "desktop", deviceKind: "desktop", complete })).toBe("tablet");
  expect(chooseShell({ override: null, preference: "desktop", deviceKind: "tablet", complete })).toBe("desktop");
  expect(chooseShell({ override: null, preference: null, deviceKind: "tablet", complete })).toBe("tablet");
  expect(chooseShell({ override: null, preference: null, deviceKind: "phone", complete })).toBe("tablet");
  expect(chooseShell({ override: null, preference: null, deviceKind: "desktop", complete })).toBe("desktop");
});

// A shell with gaps hides features: no device gets it unless chosen.
test("a tablet gets the tablet shell by default only once it is complete", () => {
  expect(chooseShell({ override: null, preference: null, deviceKind: "tablet", complete: () => false })).toBe("desktop");
  expect(chooseShell({ override: null, preference: "tablet", deviceKind: "tablet", complete: () => false })).toBe("tablet");
  expect(shellComplete("desktop")).toBe(true);
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

// The person picks the layout: both offered, the one in force checked, the
// pick kept as the device's preference.
test("the layout items check the shell in force and keep a pick", () => {
  const picks: unknown[] = [];
  const items = layoutItems("tablet", (v) => picks.push(v));
  expect(items.map((i) => [i.id, i.checked])).toEqual([["shell.desktop", false], ["shell.tablet", true]]);
  void items[0]!.run!();
  expect(picks).toEqual(["desktop"]);
});
