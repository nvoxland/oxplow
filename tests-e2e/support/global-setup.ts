// Build the binaries the suite runs, once, and hand their paths to the
// workers through the environment.
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { join } from "node:path";

import { LOG_DIR } from "./daemon.js";

/** The executable cargo built for `bin` (`--message-format=json`). */
function built(output: string, bin: string): string {
  for (const line of output.split("\n")) {
    if (!line.startsWith("{")) continue;
    const msg = JSON.parse(line) as { reason?: string; target?: { name?: string; kind?: string[] }; executable?: string | null };
    if (msg.reason === "compiler-artifact" && msg.target?.name === bin && msg.executable) return msg.executable;
  }
  throw new Error(`cargo built no ${bin}`);
}

/** The spec files under `dir` that name a timer — Playwright's
 *  `waitForTimeout` or a `setTimeout`, called or not (`test.setTimeout`
 *  is a spec's time limit, not a sleep). Specs wait on what
 *  they expect (web-first `expect`, `waitForModels`, `until`), never on
 *  time: a sleep is either too short (flaky) or too long. */
export function sleepsIn(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir).sort()) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...sleepsIn(path));
    else if (/\.ts$/.test(name) && /\bwaitForTimeout\b|(?<!test\.)\bsetTimeout\b/.test(readFileSync(path, "utf8"))) out.push(path);
  }
  return out;
}

export default function globalSetup(): void {
  const sleeps = sleepsIn(join(import.meta.dirname, "..", "specs"));
  if (sleeps.length > 0) throw new Error(`these specs sleep; wait on what they expect instead:\n${sleeps.join("\n")}`);
  rmSync(LOG_DIR, { recursive: true, force: true });
  const output = execFileSync(
    "cargo",
    ["build", "-p", "oxplow-daemon-sim", "-p", "oxplow-acp-fake", "-p", "oxplow-provider-fake", "--message-format=json"],
    { encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] },
  );
  process.env.OXPLOW_E2E_DAEMON = built(output, "oxplow-daemon-sim");
  process.env.OXPLOW_E2E_ACP_FAKE = built(output, "oxplow-acp-fake");
  process.env.OXPLOW_E2E_PROVIDER_FAKE = built(output, "oxplow-provider-fake");
}
