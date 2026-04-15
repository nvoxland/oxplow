import { mkdtempSync, writeFileSync, rmSync, existsSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export interface SessionFiles {
  dir: string;
  /** Absolute path to hooks.json — pass to claude as `--settings <path>`. */
  settingsPath: string;
  /** Absolute path to hook-forward.sh — referenced from hooks.json. */
  forwarderPath: string;
}

export interface SessionFilesOptions {
  daemonPort: number;
}

const HOOK_EVENTS = [
  "PreToolUse",
  "PostToolUse",
  "UserPromptSubmit",
  "SessionStart",
  "SessionEnd",
  "Stop",
  "Notification",
] as const;

export function createSessionFiles(opts: SessionFilesOptions): SessionFiles {
  const dir = mkdtempSync(join(tmpdir(), "newde-session-"));

  const forwarderPath = join(dir, "hook-forward.sh");
  writeFileSync(forwarderPath, buildForwarderScript(opts.daemonPort), "utf8");
  chmodSync(forwarderPath, 0o755);

  const settingsPath = join(dir, "hooks.json");
  writeFileSync(
    settingsPath,
    JSON.stringify(buildHookSettings(forwarderPath), null, 2) + "\n",
    "utf8",
  );

  return { dir, settingsPath, forwarderPath };
}

export function destroySessionFiles(s: SessionFiles): void {
  if (!existsSync(s.dir)) return;
  rmSync(s.dir, { recursive: true, force: true });
}

function buildForwarderScript(daemonPort: number): string {
  return `#!/usr/bin/env bash
event="$1"
exec curl -sS --max-time 2 \\
  -X POST \\
  -H "content-type: application/json" \\
  --data-binary @- \\
  "http://127.0.0.1:${daemonPort}/api/hook/$event" \\
  >/dev/null 2>&1 || true
`;
}

function buildHookSettings(forwarderPath: string) {
  const hooks: Record<string, unknown> = {};
  for (const event of HOOK_EVENTS) {
    const command = `${forwarderPath} ${event}`;
    if (event === "PreToolUse" || event === "PostToolUse") {
      hooks[event] = [{ matcher: "*", hooks: [{ type: "command", command }] }];
    } else {
      hooks[event] = [{ hooks: [{ type: "command", command }] }];
    }
  }
  return { hooks };
}
