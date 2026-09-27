import { expect, test } from "bun:test";
import { squarify } from "./squarify.js";

test("squarify fills the rectangle with areas proportional to values", () => {
  const out = squarify(
    [
      { value: 6, payload: "a" },
      { value: 3, payload: "b" },
      { value: 1, payload: "c" },
    ],
    0,
    0,
    100,
    50,
  );
  expect(out.map((o) => o.payload).sort()).toEqual(["a", "b", "c"]);
  const area = (p: string) => {
    const r = out.find((o) => o.payload === p)!;
    return r.w * r.h;
  };
  expect(area("a")).toBeCloseTo(3000, 0);
  expect(area("b")).toBeCloseTo(1500, 0);
  expect(area("c")).toBeCloseTo(500, 0);
  for (const r of out) {
    expect(r.x).toBeGreaterThanOrEqual(0);
    expect(r.y).toBeGreaterThanOrEqual(0);
    expect(r.x + r.w).toBeLessThanOrEqual(100.0001);
    expect(r.y + r.h).toBeLessThanOrEqual(50.0001);
  }
});

test("nothing to lay out gives nothing", () => {
  expect(squarify([], 0, 0, 10, 10)).toEqual([]);
  expect(squarify([{ value: 0, payload: 1 }], 0, 0, 10, 10)).toEqual([]);
});
