import { expect, test } from "bun:test";
import { followDensity } from "./coarsePointer.js";

// A touch screen's density follows the pointer: a finger makes it `touch`,
// a mouse (a keyboard and trackpad attached) takes it away.
test("the density follows the main pointer", () => {
  const attrs = new Map<string, string>();
  const root = {
    setAttribute: (n: string, v: string) => void attrs.set(n, v),
    removeAttribute: (n: string) => void attrs.delete(n),
  };
  let onChange: (() => void) | null = null;
  const list = {
    matches: true,
    addEventListener: (_: string, f: () => void) => (onChange = f),
    removeEventListener: () => (onChange = null),
  } as unknown as MediaQueryList & { matches: boolean };
  const stop = followDensity(root, () => list);
  expect(attrs.get("data-density")).toBe("touch");
  (list as { matches: boolean }).matches = false;
  onChange!();
  expect(attrs.has("data-density")).toBe(false);
  stop();
  expect(onChange).toBeNull();
});
