import { expect, test } from "bun:test";

import { drawsFor } from "./actionPointDraw.js";

test("an action point draws nothing with no contributor, a button with one, a menu with several", () => {
  expect(drawsFor(0)).toBe("none");
  expect(drawsFor(1)).toBe("button");
  expect(drawsFor(2)).toBe("menu");
  expect(drawsFor(5)).toBe("menu");
});
