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
// transport: the `api.ts` facade + the xterm pane that pipes the user's
// own keystrokes/paste. Generated bindings and test files are excluded
// from the scan entirely (below), so they need no entry here.
const ALLOWED = new Set<string>(["api.ts", join("components", "TerminalPane.tsx")]);

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
});
