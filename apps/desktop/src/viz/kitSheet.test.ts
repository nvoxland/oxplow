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

// A row a click acts on (`ox-clickable`: a table row that picks or opens, a
// tree row that opens a folder) shows the pointer wherever it is drawn, so
// a folder row's cursor matches the file link beside it.
test("a clickable row shows the pointer in a table and in a tree", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  expect(sheet).toMatch(/(^|[},]\s*)\.ox-clickable\s*\{[^}]*cursor:\s*pointer/);
  expect(sheet).not.toMatch(/\.ox-table tr\.ox-clickable\s*\{[^}]*cursor/);
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

// A section inside an embedded page is its own frame's first, but the
// embed's frame may follow a table or tree: it keeps its gap above then.
test("a section in a frame nested in a later frame keeps its space above", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  const rule = /\[data-node\]:not\(:first-child\)\s+\[data-node\]\s*>\s*\.ox-section\s*\{[^}]*margin-top:\s*var\(--space-6\)/;
  expect(sheet).toMatch(rule);
  expect(sheet.search(rule)).toBeGreaterThan(sheet.search(/\[data-node\]:first-child\s*>\s*\.ox-section/));
});

// A rail group's body sits a little in from its heading, the same for
// every group (Filters, Contents, See also): one rule on the group.
test("every rail group's body is indented from its heading", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  expect(sheet).toMatch(/\.ox-rail-group\s*>\s*:not\(\.ox-rail-group-head\)\s*\{[^}]*margin-left:\s*var\(--space-3\)/);
});

// A rail panel's part (`.ox-compact`) is narrow: its sections sit close, a
// detail's facts are short lines with no rules, and a fact's buttons wrap
// under its value as small buttons whose labels never break.
test("a compact part is tight: close sections, ruleless facts, buttons that wrap whole", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  expect(sheet).toMatch(/\.ox-compact [^{]*\.ox-section[^{]*\{[^}]*margin-top:\s*var\(--space-2\)/);
  expect(sheet).toMatch(/\.ox-compact \.ox-fact dt,\s*\.ox-compact \.ox-fact dd\s*\{[^}]*border-bottom:\s*0/);
  expect(sheet).toMatch(/\.ox-compact \.ox-fact dd\s*\{[^}]*flex-wrap:\s*wrap/);
  expect(sheet).toMatch(/\.ox-compact \.ox-fact-actions \.ox-button\s*\{[^}]*white-space:\s*nowrap/);
});

// A component that draws its own menu (the Tasks board's card menu) draws it
// with the kit's: a raised list over the page, its items full-width buttons
// that light up under the pointer and the keyboard, a rule between groups
// and a danger look for a delete.
test("the kit has a menu: a raised list of items, separators and a danger item", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  expect(sheet).toMatch(/\.ox-menu\s*\{[^}]*position:\s*fixed[^}]*background:\s*var\(--surface-elevated\)/);
  expect(sheet).toMatch(/\.ox-menu-item\s*\{[^}]*width:\s*100%/);
  expect(sheet).toMatch(/\.ox-menu-item:hover,\s*\.ox-menu-item:focus-visible\s*\{[^}]*background:\s*var\(--accent-soft-bg\)/);
  expect(sheet).toMatch(/\.ox-menu-separator\s*\{[^}]*border-top:\s*1px solid var\(--border-subtle\)/);
  expect(sheet).toMatch(/\.ox-menu-item\.ox-danger\s*\{[^}]*color:\s*var\(--severity-critical\)/);
});

// A component's frame scrolls with the app's thin scrollbars (index.html),
// not WebKit's default bar, from the theme's tokens.
test("the kit draws the app's thin scrollbars", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  expect(sheet).toMatch(/::-webkit-scrollbar\s*\{[^}]*width:\s*10px/);
  expect(sheet).toMatch(/::-webkit-scrollbar-track[^{]*\{[^}]*background:\s*transparent/);
  expect(sheet).toMatch(/::-webkit-scrollbar-thumb\s*\{[^}]*background:\s*var\(--border-subtle\)/);
  expect(sheet).toMatch(/::-webkit-scrollbar-thumb:hover\s*\{[^}]*background:\s*var\(--border-strong\)/);
});

// A list's parent and its children (a `depth` list, the board's epics) are
// one raised panel: its top line the parent, its bottom edge where they end.
test("the kit has a nesting panel", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  expect(sheet).toMatch(/\.ox-nest\s*\{[^}]*background:\s*var\(--surface-elevated\)[^}]*border:\s*1px solid var\(--border-strong\)[^}]*border-radius:\s*var\(--radius-md\)/);
});

// A param value you type has no caret: it reads as a text box.
test("a typed param value draws no caret", () => {
  const sheet = readFileSync(SHEET, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
  expect(sheet).toMatch(/\.ox-param-value\.ox-param-typed::after\s*\{[^}]*content:\s*none/);
});
