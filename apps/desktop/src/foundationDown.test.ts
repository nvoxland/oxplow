import { expect, test } from "bun:test";

import type { Extension } from "./api.js";
import { FOUNDATION, foundationDown } from "./foundationDown.js";

const ext = (name: string, enabled: boolean, errors: string[] = []) =>
  ({ name, enabled, errors }) as unknown as Extension;

test("nothing is reported before the listing loads", () => {
  expect(foundationDown(null)).toBeNull();
});

test("a running foundation is not reported", () => {
  expect(foundationDown([ext(FOUNDATION, true), ext("other", false, ["x"])])).toBeNull();
});

test("a held foundation reports its own errors", () => {
  expect(foundationDown([ext(FOUNDATION, false, ["a", "b"])])).toEqual(["a", "b"]);
});

test("a held foundation without errors still reports a line", () => {
  expect(foundationDown([ext(FOUNDATION, false)])?.length).toBe(1);
});

test("a loaded listing without foundation reports it missing", () => {
  expect(foundationDown([ext("other", true)])).toEqual(["oxplow-foundation isn't in the extension listing."]);
});

test("an empty listing (one that failed to load) is not reported", () => {
  expect(foundationDown([])).toBeNull();
});
