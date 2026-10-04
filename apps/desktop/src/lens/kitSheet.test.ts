import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// tsk961: the component kit's stylesheet (served at
// `/component-lib/oxplow-kit.css`) styles only through the theme: every
// `var(--x)` it reads is a token the app's root defines — the ones
// `applyTheme` sets on a frame's root — and it names no color of its own.

const SHEET = join(import.meta.dir, "..", "..", "..", "..", "crates", "oxplow-daemon", "assets", "oxplow-kit.css");
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
