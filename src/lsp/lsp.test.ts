import { afterEach, describe, expect, test } from "bun:test";
import { lspLanguageIdForPath, registerLanguageServer, unregisterLanguageServer } from "./lsp.js";

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

describe("registerLanguageServer", () => {
  afterEach(() => {
    unregisterLanguageServer("python");
    unregisterLanguageServer("rust");
  });

  test("registered extensions resolve to the custom languageId", () => {
    registerLanguageServer({
      languageId: "python",
      extensions: [".py"],
      command: "pyright-langserver",
      args: ["--stdio"],
    });
    expect(lspLanguageIdForPath("src/app.py")).toBe("python");
  });

  test("matches extensions case-insensitively", () => {
    registerLanguageServer({
      languageId: "rust",
      extensions: [".rs"],
      command: "rust-analyzer",
      args: [],
    });
    expect(lspLanguageIdForPath("SRC/Main.RS")).toBe("rust");
  });

  test("re-registering the same languageId replaces the prior entry", () => {
    registerLanguageServer({
      languageId: "python",
      extensions: [".py"],
      command: "pyls-v1",
      args: [],
    });
    registerLanguageServer({
      languageId: "python",
      extensions: [".py", ".pyi"],
      command: "pyls-v2",
      args: [],
    });
    expect(lspLanguageIdForPath("src/app.py")).toBe("python");
    expect(lspLanguageIdForPath("src/app.pyi")).toBe("python");
  });
});
