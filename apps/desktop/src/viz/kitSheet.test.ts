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

// A column's wrap class (`columnWrap`) decides its cells: dates and short
// values stay on a line, words break between words only, and only a column
// of long tokens breaks anywhere.
test("a table column wraps by its class", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  expect(sheet).toMatch(/\.ox-table td\.ox-nowrap\s*\{[^}]*white-space:\s*nowrap/);
  expect(sheet).toMatch(/\.ox-table td\.ox-words\s*\{[^}]*overflow-wrap:\s*break-word/);
  expect(sheet).toMatch(/\.ox-table td\.ox-breakable\s*\{[^}]*overflow-wrap:\s*anywhere/);
  expect(sheet).not.toMatch(/\.ox-table td\s*\{[^}]*overflow-wrap/);
  expect(sheet).not.toMatch(/td:first-child/);
});

// Rows are striped by default; hover, selection, the cursor and emphasis
// are declared after the stripe, so they win.
test("striped rows give way to hover, selection, cursor and emphasis", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  expect(sheet).toMatch(/\.ox-table tr\.ox-stripe\s*\{[^}]*background:\s*var\(--surface-row-stripe\)/);
  const stripe = sheet.indexOf(".ox-table tr.ox-stripe");
  for (const later of [".ox-table tr.ox-clickable:hover", ".ox-table:focus-within tr[data-cursor", ".ox-table tr.ox-emphasis"]) {
    expect(sheet.indexOf(later)).toBeGreaterThan(stripe);
  }
});

// Sections stack apart, but the first one starts at the top of its page:
// a margin above it is a gap under the toolbar. A page's section is the
// first child of its node's frame (`[data-node]`), so the first section is
// the one whose frame is first — every section is its own frame's first.
test("only the first section has no space above it", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  expect(sheet).toMatch(/\[data-node\]:first-child\s*>\s*\.ox-section[^{]*\{[^}]*margin-top:\s*0/);
  expect(sheet).not.toMatch(/(^|[},]\s*)\.ox-section:first-child\s*[,{]/);
});

// A rail group's body sits a little in from its heading, the same for
// every group (Filters, Contents, See also): one rule on the group.
test("every rail group's body is indented from its heading", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  expect(sheet).toMatch(/\.ox-rail-group\s*>\s*:not\(\.ox-rail-group-head\)\s*\{[^}]*margin-left:\s*var\(--space-3\)/);
});
