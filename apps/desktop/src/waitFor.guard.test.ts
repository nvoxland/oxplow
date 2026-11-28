import { expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

// A test that waits for an element to go waits on a boolean:
// `waitFor(() => expect(view.queryByTestId("x") === null).toBe(true))`.
// `expect(element).toBeNull()` fails on each poll while the element is still
// there, and bun writes the whole happy-dom element into that failure's
// message: about 800 ms a poll on a laptop and over 5 s on CI's runner, so
// the test outlasts its time limit there.

const SRC = dirname(fileURLToPath(import.meta.url));

function tests(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "generated" || name === "node_modules" ? [] : tests(path);
    return /\.test\.tsx?$/.test(name) ? [path] : [];
  });
}

const SLOW_WAIT = /waitFor\(\s*(?:async\s*)?\(\)\s*=>\s*\{?[^;{}]*?expect\([^;]*?query(?:All)?By[^;]*?\)\.(?:toBeNull|toBeFalsy)\(\)/;

test("no test waits for an element to go with expect(element).toBeNull()", () => {
  // This file names the slow form, as an example.
  const others = tests(SRC).filter((file) => file !== fileURLToPath(import.meta.url));
  const sites = others.flatMap((file) =>
    readFileSync(file, "utf8")
      .split("\n")
      .flatMap((line, i) => (SLOW_WAIT.test(line) ? [`${relative(SRC, file)}:${i + 1}`] : [])),
  );
  expect(sites).toEqual([]);
});

test("the rule finds the slow form", () => {
  expect(SLOW_WAIT.test('await waitFor(() => expect(view.queryByTestId("x")).toBeNull());')).toBe(true);
  expect(SLOW_WAIT.test('await waitFor(() => expect(view.queryByTestId("x") === null).toBe(true));')).toBe(false);
});
