import { expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

// tsk630: `window.confirm` / `alert` / `prompt` block the renderer (and
// Tauri's webview answers `prompt` with null), so `.context/usability.md`
// rules them out: a destructive button asks with `InlineConfirm`, a
// command run from nowhere through `personCommands`' `CommandConfirm`, a
// failed operation through `recordOpError`.

const SRC_DIR = import.meta.dir;

function sourceFiles(): string[] {
  return readdirSync(SRC_DIR, { recursive: true })
    .map((p) => String(p))
    .filter((p) => p.endsWith(".ts") || p.endsWith(".tsx"))
    .filter((p) => !p.endsWith(".test.ts") && !p.endsWith(".test.tsx"))
    .filter((p) => !p.includes(join("tauri-bridge", "generated")));
}

test("no renderer source calls a blocking browser dialog", () => {
  const calls = /\bwindow\.(confirm|alert|prompt)\s*\(/;
  const offenders = sourceFiles()
    .filter((rel) => calls.test(readFileSync(join(SRC_DIR, rel), "utf8")))
    .sort();
  expect(offenders).toEqual([]);
});
