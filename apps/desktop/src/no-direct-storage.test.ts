import { expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

// Page state has four homes (`.context/pages-and-tabs.md` "Page state"):
// a page's view is its tab's view params, its own memory its history
// state, what it or the app remembers for the person page memory and app
// preferences (`prefs/remembered.ts`). The browser's storage holds only
// the shell's own keys, behind `shellStorage.ts` — the one file that may
// touch it.

const SRC_DIR = import.meta.dir;
const ALLOWED = new Set(["shellStorage.ts"]);
const STORES = ["localStorage", "sessionStorage", "indexedDB"] as const;

function sourceFiles(): string[] {
  return readdirSync(SRC_DIR, { recursive: true })
    .map((p) => String(p))
    .filter((p) => p.endsWith(".ts") || p.endsWith(".tsx"))
    .filter((p) => !p.endsWith(".test.ts") && !p.endsWith(".test.tsx"))
    .filter((p) => !p.includes(join("tauri-bridge", "generated")));
}

/** `text` without its comments, so prose that names a store isn't a use. */
function code(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:"'`])\/\/.*$/gm, "$1");
}

/** The browser stores `src` reaches for. */
function storesUsed(src: string): string[] {
  const body = code(src);
  return STORES.filter((name) => new RegExp(`(?<![\\w$])${name}(?![\\w$])`).test(body));
}

test("the scan finds a store however it is reached, and not one named in a comment", () => {
  expect(storesUsed("window.localStorage.getItem('k')")).toEqual(["localStorage"]);
  expect(storesUsed("const s = sessionStorage;")).toEqual(["sessionStorage"]);
  expect(storesUsed("const { localStorage } = window;")).toEqual(["localStorage"]);
  expect(storesUsed("globalThis.indexedDB.open('x')")).toEqual(["indexedDB"]);
  expect(storesUsed("// localStorage is the shell's\n/* sessionStorage */")).toEqual([]);
  expect(storesUsed("const myLocalStorageKey = 1;")).toEqual([]);
});

// Moving to their homes (`.context/pages-and-tabs.md` "Page state"); this
// list only shrinks.
const NOT_MOVED_YET = [
  "components/Plan/TasksFilterBar.tsx",
  "components/Plan/tasks-scope.ts",
];

test("only shellStorage.ts touches the browser's storage", () => {
  const offenders = sourceFiles()
    .filter((rel) => !ALLOWED.has(rel))
    .filter((rel) => storesUsed(readFileSync(join(SRC_DIR, rel), "utf8")).length > 0)
    .sort();
  expect(offenders).toEqual(NOT_MOVED_YET);
});
