// Build the binaries the suite runs, once, and hand their paths to the
// workers through the environment.
import { execFileSync } from "node:child_process";

/** The executable cargo built for `bin` (`--message-format=json`). */
function built(output: string, bin: string): string {
  for (const line of output.split("\n")) {
    if (!line.startsWith("{")) continue;
    const msg = JSON.parse(line) as { reason?: string; target?: { name?: string; kind?: string[] }; executable?: string | null };
    if (msg.reason === "compiler-artifact" && msg.target?.name === bin && msg.executable) return msg.executable;
  }
  throw new Error(`cargo built no ${bin}`);
}

export default function globalSetup(): void {
  const output = execFileSync(
    "cargo",
    ["build", "-p", "oxplow-daemon-sim", "-p", "oxplow-acp-fake", "--message-format=json"],
    { encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] },
  );
  process.env.OXPLOW_E2E_DAEMON = built(output, "oxplow-daemon-sim");
  process.env.OXPLOW_E2E_ACP_FAKE = built(output, "oxplow-acp-fake");
}
