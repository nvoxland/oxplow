import { describe, expect, test } from "bun:test";
import { titleInitials } from "./initials.js";

describe("titleInitials", () => {
  test("multi-word → uppercase initials of the first two words", () => {
    expect(titleInitials("Git Dashboard")).toBe("GD");
    expect(titleInitials("the quick brown fox")).toBe("TQ");
  });

  test("single word ≥2 chars → first char upper, second lower", () => {
    expect(titleInitials("oxplow")).toBe("Ox");
    expect(titleInitials("BUILD")).toBe("Bu");
  });

  test("single character → uppercased", () => {
    expect(titleInitials("x")).toBe("X");
  });

  test("empty / whitespace → '?'", () => {
    expect(titleInitials("")).toBe("?");
    expect(titleInitials("   ")).toBe("?");
  });

  test("collapses leading/inner whitespace before picking words", () => {
    expect(titleInitials("  Terminal 2 ")).toBe("T2");
  });
});
