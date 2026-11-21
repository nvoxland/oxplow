import { afterEach, expect, test } from "bun:test";

import { languageForPath, setDeclaredEditorLanguages, allKnownLanguageIds } from "./editor-language.js";

afterEach(() => setDeclaredEditorLanguages([]));

test("an extension no pack declares falls back to the table of well-known ones", () => {
  expect(languageForPath("src/a.kt")).toBe("kotlin");
  expect(languageForPath("README")).toBe("plaintext");
  expect(languageForPath(null)).toBe("plaintext");
});

test("a declared language names its files, and wins over the fallback", () => {
  setDeclaredEditorLanguages([
    { extensions: ["zz"], monaco: "zeta" },
    { extensions: ["kt"], monaco: "kotlin-pack" },
  ]);
  expect(languageForPath("lib/a.ZZ")).toBe("zeta");
  expect(languageForPath("src/a.kt")).toBe("kotlin-pack");
  expect(languageForPath("src/a.rs")).toBe("rust");
  expect(allKnownLanguageIds()).toContain("zeta");
});
