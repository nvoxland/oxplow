import { expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

// A page on plain HTTP is not a secure context: `crypto.randomUUID` and
// `navigator.clipboard` do not exist there. `newId.ts` and `clipboard.ts`
// are the only code that may name them.

const SRC_DIR = import.meta.dir;
const HELPERS = ["newId.ts", "clipboard.ts"];

function sourceFiles(): string[] {
  return readdirSync(SRC_DIR, { recursive: true })
    .map((p) => String(p))
    .filter((p) => p.endsWith(".ts") || p.endsWith(".tsx"))
    .filter((p) => !p.endsWith(".test.ts") && !p.endsWith(".test.tsx"))
    .filter((p) => !p.includes(join("tauri-bridge", "generated")))
    .filter((p) => !HELPERS.includes(p));
}

function code(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:"'`])\/\/.*$/gm, "$1");
}

/** The secure-context-only APIs `src` uses, comments aside. */
function insecureApis(src: string): string[] {
  const body = code(src);
  const found: string[] = [];
  if (/\brandomUUID\b/.test(body)) found.push("crypto.randomUUID");
  if (/\bnavigator\s*\??\.\s*clipboard\b/.test(body)) found.push("navigator.clipboard");
  return found;
}

test("the scan catches both APIs, however they are reached, and not prose", () => {
  expect(insecureApis("const id = crypto.randomUUID();")).toEqual(["crypto.randomUUID"]);
  expect(insecureApis("globalThis.crypto?.randomUUID?.()")).toEqual(["crypto.randomUUID"]);
  expect(insecureApis("await navigator.clipboard.writeText(t)")).toEqual(["navigator.clipboard"]);
  expect(insecureApis("navigator.clipboard?.readText()")).toEqual(["navigator.clipboard"]);
  expect(insecureApis("// uses navigator.clipboard\nconst x = newId();")).toEqual([]);
  expect(insecureApis("copyText(text); newId();")).toEqual([]);
});

test("no renderer source uses crypto.randomUUID or navigator.clipboard outside the helpers", () => {
  const offenders = sourceFiles()
    .filter((rel) => insecureApis(readFileSync(join(SRC_DIR, rel), "utf8")).length > 0)
    .sort();
  expect(offenders).toEqual([]);
});
