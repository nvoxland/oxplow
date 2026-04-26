import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchOxplow, probeLog, runProbe } from "./harness.ts";
const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });
  const { window, close } = await launchOxplow(projectDir);
  try {
    await window.waitForTimeout(3_000);
    await window.getByTestId("rail-page-files").click();
    await window.getByTestId("page-files").waitFor({ state: "visible", timeout: 5_000 });
    await window.waitForTimeout(400);
    await window.getByTestId("files-commit").click();
    await window.waitForTimeout(500);
    await window.getByTestId("files-commit-message").fill(`Expose data-agent-status on AgentStatusDot

Adds data-agent-status={status} and data-agent-label={LABELS[status]}
to the <span> so probes can query agent state from outside the
active tab (document.querySelector("[data-agent-status]") now
returns a reliable signal). Surfaced by fix-20260419-220229; scope
kept to one file, two new attributes.

Done by the inner agent during a /self-ralph dogfood run; outer
agent approved via oxplow's Files commit dialog.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`);
    await window.getByTestId("files-commit-submit").click();
    probeLog("[as-approve] submitted");
    await window.waitForTimeout(3_000);
    await window.screenshot({ path: resolve(outDir, "as-approve-done.png") });
  } finally {
    await close();
  }
}

runProbe("dogfood-as-approve", main, { wallMs: 60_000, silenceMs: 45_000 }).catch(() => process.exit(1));
