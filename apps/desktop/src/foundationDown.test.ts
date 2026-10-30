import { expect, test } from "bun:test";

import type { Extension } from "./api.js";
import { FOUNDATION, foundationDown, foundationDownText } from "./foundationDown.js";

// Each error a diagnostic with no file: it reads as its message.
const ext = (name: string, enabled: boolean, errors: string[] = []) =>
  ({
    name,
    enabled,
    path: `extensions/${name}`,
    source: null,
    errors: errors.map((message) => ({ severity: "error", code: "manifest.parse", file: null, line: null, contribution: null, message, fix: null, mismatch: null })),
  }) as unknown as Extension;

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

test("the dialog shows the held reason and no developer command", () => {
  const text = foundationDownText(["held: needs 0.8"]).join("\n");
  expect(text).toContain("held: needs 0.8");
  expect(text).not.toContain("cargo");
  expect(text).not.toContain("oxplow-dev");
});
