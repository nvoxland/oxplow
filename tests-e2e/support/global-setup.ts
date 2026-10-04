// Build the binaries the suite runs, once, and hand their paths to the
// workers through the environment.
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

/** The executable cargo built for `bin` (`--message-format=json`). */
function built(output: string, bin: string): string {
  for (const line of output.split("\n")) {
    if (!line.startsWith("{")) continue;
    const msg = JSON.parse(line) as { reason?: string; target?: { name?: string; kind?: string[] }; executable?: string | null };
    if (msg.reason === "compiler-artifact" && msg.target?.name === bin && msg.executable) return msg.executable;
  }
  throw new Error(`cargo built no ${bin}`);
}

/** Specs wait on what they expect (web-first `expect`, `waitForModels`),
 *  never on time: a sleep is either too short (flaky) or too long. */
function refuseSleeps(dir: string): void {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) refuseSleeps(path);
    else if (/\.ts$/.test(name) && readFileSync(path, "utf8").includes("waitForTimeout(")) {
      throw new Error(`${path} sleeps (waitForTimeout); wait on what it expects instead`);
    }
  }
}

export default function globalSetup(): void {
  refuseSleeps(join(import.meta.dirname, "..", "specs"));
  const output = execFileSync(
    "cargo",
    ["build", "-p", "oxplow-daemon-sim", "-p", "oxplow-acp-fake", "-p", "oxplow-provider-fake", "--message-format=json"],
    { encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] },
  );
  process.env.OXPLOW_E2E_DAEMON = built(output, "oxplow-daemon-sim");
  process.env.OXPLOW_E2E_ACP_FAKE = built(output, "oxplow-acp-fake");
  process.env.OXPLOW_E2E_PROVIDER_FAKE = built(output, "oxplow-provider-fake");
}
