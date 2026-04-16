import { mkdtempSync, writeFileSync, rmSync, existsSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { PaneKind } from "./stream-store.js";

export interface SessionFiles {
  dir: string;
  /** Absolute path to hooks.json — pass to claude as `--settings <path>`. */
  settingsPath: string;
  /** Absolute path to hook-forward.sh — referenced from hooks.json. */
  forwarderPath: string;
}

export interface SessionFilesOptions {
  daemonPort: number;
  streamId: string;
  pane: PaneKind;
}

export interface ElectronSessionFilesOptions {
  hookInboxDir: string;
  streamId: string;
  batchId: string;
  pane?: PaneKind;
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
  writeFileSync(forwarderPath, buildForwarderScript(opts), "utf8");
  chmodSync(forwarderPath, 0o755);

  const settingsPath = join(dir, "hooks.json");
  writeFileSync(
    settingsPath,
    JSON.stringify(buildHookSettings(forwarderPath), null, 2) + "\n",
    "utf8",
  );

  return { dir, settingsPath, forwarderPath };
}

export function createElectronSessionFiles(opts: ElectronSessionFilesOptions): SessionFiles {
  const dir = mkdtempSync(join(tmpdir(), "newde-session-"));

  const forwarderPath = join(dir, "hook-forward.sh");
  writeFileSync(forwarderPath, buildElectronForwarderScript(opts), "utf8");
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

function buildForwarderScript(opts: SessionFilesOptions): string {
  const query = new URLSearchParams({ stream: opts.streamId, pane: opts.pane }).toString();
  return `#!/usr/bin/env bash
event="$1"
curl -sS --max-time 2 \\
  -X POST \\
  -H "content-type: application/json" \\
  --data-binary @- \\
  "http://127.0.0.1:${opts.daemonPort}/api/hook/$event?${query}" \\
  >/dev/null 2>&1 || true
exit 0
`;
}

function buildElectronForwarderScript(opts: ElectronSessionFilesOptions): string {
  return `#!/usr/bin/env bash
event="$1"
node - "$event" "${escapeShellArg(opts.streamId)}" "${escapeShellArg(opts.batchId)}" "${escapeShellArg(opts.pane ?? "")}" "${escapeShellArg(opts.hookInboxDir)}" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");

const [event, streamId, batchId, pane, dir] = process.argv.slice(2);
let body = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  body += chunk;
});
process.stdin.on("end", () => {
  let payload;
  try {
    payload = body ? JSON.parse(body) : {};
  } catch {
    payload = { raw: body };
  }
  const file = path.join(dir, \`\${Date.now()}-\${process.pid}-\${crypto.randomBytes(4).toString("hex")}.json\`);
  try {
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(file, JSON.stringify({ event, streamId, batchId, pane: pane || undefined, payload }));
  } catch {}
  process.exit(0);
});
process.stdin.resume();
NODE
exit 0
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

function escapeShellArg(value: string): string {
  return value.replace(/'/g, `'\\''`);
}
