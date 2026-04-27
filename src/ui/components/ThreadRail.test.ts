import { describe, expect, test } from "bun:test";
import { nextThreadTitle } from "./ThreadRail.js";

describe("nextThreadTitle", () => {
  test("renders Thread N for the supplied next index", () => {
    expect(nextThreadTitle(1)).toBe("Thread 1");
    expect(nextThreadTitle(7)).toBe("Thread 7");
  });

  test("bumps cleanly across the Save-and-Another reset (called with cursor + 1)", () => {
    // The hook bumps the cursor before calling nextThreadTitle so two
    // consecutive Save-and-Another submissions don't ship identical
    // placeholder titles.
    const first = nextThreadTitle(3);
    const second = nextThreadTitle(4);
    expect(first).not.toBe(second);
    expect(second).toBe("Thread 4");
  });
});
