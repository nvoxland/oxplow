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

const DIALOGS = ["confirm", "alert", "prompt"] as const;

/** `text` without its comments (line and block), so prose isn't a call. */
function code(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:"'`])\/\/.*$/gm, "$1");
}

/** The blocking dialogs `src` calls (tsk899): through `window.` /
 *  `globalThis.` / `self.`, destructured off one of those, or bare — unless
 *  the file binds that name itself (a local `confirm()` isn't the
 *  browser's). */
function dialogCalls(src: string): string[] {
  const body = code(src);
  return DIALOGS.filter((name) => {
    const qualified = new RegExp(`\\b(window|globalThis|self)\\.${name}\\s*\\(`);
    const destructured = new RegExp(`\\{[^}]*\\b${name}\\b[^}]*\\}\\s*=\\s*(window|globalThis|self)\\b`);
    const bare = new RegExp(`(?<![\\w.$])${name}\\s*\\(`);
    const bound = new RegExp(
      `\\b(function|const|let|var)\\s+${name}\\b|^\\s*(async\\s+)?${name}\\s*\\([^)]*\\)\\s*[:{]|[{,]\\s*${name}\\s*[,}:]`,
      "m",
    );
    return qualified.test(body) || destructured.test(body) || (bare.test(body) && !bound.test(body));
  });
}

test("the scan catches every way of calling a dialog, and not a local one", () => {
  expect(dialogCalls("window.confirm('x')")).toEqual(["confirm"]);
  expect(dialogCalls("if (globalThis.alert('x')) {}")).toEqual(["alert"]);
  expect(dialogCalls("const ok = confirm('sure?');")).toEqual(["confirm"]);
  expect(dialogCalls("const { prompt } = window;\nprompt('name');")).toEqual(["prompt"]);
  expect(dialogCalls("function confirm(row) {}\nconfirm(row);")).toEqual([]);
  expect(dialogCalls("bridge.confirm(command);")).toEqual([]);
  expect(dialogCalls("// a confirm (inline)\n")).toEqual([]);
});

test("no renderer source calls a blocking browser dialog", () => {
  const offenders = sourceFiles()
    .filter((rel) => dialogCalls(readFileSync(join(SRC_DIR, rel), "utf8")).length > 0)
    .sort();
  expect(offenders).toEqual([]);
});
