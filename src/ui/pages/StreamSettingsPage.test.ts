import { describe, expect, test } from "bun:test";
import { normalizePromptForSave } from "./StreamSettingsPage.js";

describe("normalizePromptForSave", () => {
  test("non-blank text is trimmed and returned", () => {
    expect(normalizePromptForSave("  hello world  ")).toBe("hello world");
  });

  test("empty string normalizes to null so the prompt clears", () => {
    expect(normalizePromptForSave("")).toBeNull();
  });

  test("whitespace-only input normalizes to null", () => {
    expect(normalizePromptForSave("   \n\t  ")).toBeNull();
  });
});
