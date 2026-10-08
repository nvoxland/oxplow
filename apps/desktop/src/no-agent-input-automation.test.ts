import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

// Guard for the no-automation invariant (see
// .context/agent-model.md → "No synthesized agent terminal input").
//
// The ONLY source of agent terminal input is human keystrokes / paste
// via the UI. The terminal-input transport (`forwardTerminalInput`) and
// the `{type:"input"}` protocol message it carries must stay confined to
// the human-input path, so no other renderer code can synthesize agent
// input. If this fails, you've added a non-human caller — that's the
// automation vector this guard exists to block. Route human input
// through TerminalPane; steer the agent via hook responses, never by
// typing at it.

const SRC_DIR = import.meta.dir;

// These files ARE the human-input path and are allowed to touch the
// transport: the `api.ts` facade, the xterm pane that pipes the user's
// own keystrokes/paste, and the pane's ordered sender (tsk979), which
// sends only through the call the pane hands it. Generated bindings and
// test files are excluded from the scan entirely (below), so they need no
// entry here.
const TERMINAL_PANE = join("components", "TerminalPane.tsx");
const ALLOWED = new Set<string>(["api.ts", TERMINAL_PANE, join("components", "terminalInput.ts")]);

function sourceFiles(): string[] {
  return readdirSync(SRC_DIR, { recursive: true })
    .map((p) => String(p))
    .filter((p) => p.endsWith(".ts") || p.endsWith(".tsx"))
    .filter((p) => !p.endsWith(".test.ts") && !p.endsWith(".test.tsx"))
    .filter((p) => !p.includes(join("tauri-bridge", "generated")));
}

function offenders(pattern: RegExp): string[] {
  const hits: string[] = [];
  for (const rel of sourceFiles()) {
    if (ALLOWED.has(rel)) continue;
    if (pattern.test(readFileSync(join(SRC_DIR, rel), "utf8"))) hits.push(rel);
  }
  return hits.sort();
}

describe("no agent input automation", () => {
  test("forwardTerminalInput is only referenced on the human-input path", () => {
    expect(offenders(/forwardTerminalInput/)).toEqual([]);
  });

  test('{type:"input"} terminal messages are only built on the human-input path', () => {
    expect(offenders(/type:\s*"input(-binary)?"/)).toEqual([]);
  });

  // The ordered sender carries `{type:"input"}` messages: only the pane may
  // make one, so nothing else gets a way to type at the agent through it.
  test("terminalSender is only used by the terminal pane", () => {
    const allowed = new Set([TERMINAL_PANE, join("components", "terminalInput.ts")]);
    const hits = sourceFiles()
      .filter((rel) => !allowed.has(rel))
      .filter((rel) => /\bterminalSender\b/.test(readFileSync(join(SRC_DIR, rel), "utf8")));
    expect(hits).toEqual([]);
  });

  // ACP agents (tsk281): a prompt is sent only by the prompt box, on the
  // person's Enter. `api.ts` defines the wrapper; nothing else may call it.
  test("acpPrompt is only referenced by the prompt box", () => {
    const allowed = new Set(["api.ts", join("components", "acp", "AcpPromptBox.tsx")]);
    const hits = sourceFiles()
      .filter((rel) => !allowed.has(rel))
      .filter((rel) => /\bacpPrompt\b/.test(readFileSync(join(SRC_DIR, rel), "utf8")));
    expect(hits).toEqual([]);
  });

  // Starting a session only opens its slot: the picker never types into
  // the agent it starts, nor drafts a prompt for it.
  test("the session picker sends the agent nothing", () => {
    const text = readFileSync(join(SRC_DIR, "pages", "NewSessionPage.tsx"), "utf8");
    expect(text).not.toMatch(/\b(acpPrompt|forwardTerminalInput|insertIntoAgent|submitHumanPrompt)\b/);
  });

  // A failing extension's repair (P7.C3): the person presses Repair with
  // the Agent, which fills the agent's input with one mention line and
  // sends nothing. Only that button may call it — nothing runs it on a
  // disable, a refresh or an event.
  test("repairWithAgent is only referenced by the Extensions settings button", () => {
    const allowed = new Set(["pluginHealth.ts", join("components", "ExtensionsSection.tsx")]);
    const hits = sourceFiles()
      .filter((rel) => !allowed.has(rel))
      .filter((rel) => /\brepairWithAgent\b/.test(readFileSync(join(SRC_DIR, rel), "utf8")));
    expect(hits).toEqual([]);
  });
});
