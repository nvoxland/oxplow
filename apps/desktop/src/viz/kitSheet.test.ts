import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// tsk961: the component kit's stylesheet (served at
// `/component-lib/oxplow-kit.css`) styles only through the theme: every
// `var(--x)` it reads is a token the app's root defines — the ones
// `applyTheme` sets on a frame's root — and it names no color of its own.

const SHEET = join(import.meta.dir, "..", "..", "..", "..", "crates", "oxplow-app", "assets", "component", "oxplow-kit.css");
const INDEX = join(import.meta.dir, "..", "..", "index.html");

test("every var(--x) in the kit's sheet is a theme token, and it names no color", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  const root = /:root\s*\{([\s\S]*?)\n\s*\}/.exec(readFileSync(INDEX, "utf8"))![1]!;
  const tokens = new Set([...root.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]));
  const used = [...sheet.matchAll(/var\((--[a-z0-9-]+)/g)].map((m) => m[1]!);
  expect(used.length).toBeGreaterThan(0);
  expect(used.filter((t) => !tokens.has(t))).toEqual([]);
  expect(sheet).not.toMatch(/#[0-9a-fA-F]{3,8}\b|\brgba?\(|\bhsla?\(/);
});

// A table's first column is the row's name: its words never break inside
// (a word a letter wide made "Alerts" read "Aler/ts"); the other columns'
// long words (a URI, a dotted id) still break anywhere.
test("a table's first column breaks only between words", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  expect(sheet).toMatch(/\.ox-table td:first-child\s*\{[^}]*overflow-wrap:\s*break-word/);
  expect(sheet).toMatch(/\.ox-table td\s*\{[^}]*overflow-wrap:\s*anywhere/);
});
