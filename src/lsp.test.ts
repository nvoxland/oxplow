import { describe, expect, test } from "bun:test";
import { lspLanguageIdForPath } from "./lsp.js";

describe("lspLanguageIdForPath", () => {
  test("maps TypeScript-family files", () => {
    expect(lspLanguageIdForPath("src/app.ts")).toBe("typescript");
    expect(lspLanguageIdForPath("src/view.tsx")).toBe("typescript");
  });

  test("maps JavaScript-family files", () => {
    expect(lspLanguageIdForPath("src/app.js")).toBe("javascript");
    expect(lspLanguageIdForPath("src/app.mjs")).toBe("javascript");
  });

  test("returns null for unsupported languages", () => {
    expect(lspLanguageIdForPath("README.md")).toBeNull();
  });
});
