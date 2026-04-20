// Approve pass 2's dogfood work via newde's Files commit dialog.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchNewde, probeLog, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchNewde(projectDir);
  try {
    await window.waitForTimeout(3_000);
    await window.getByTestId("dock-tab-project").click();
    await window.waitForTimeout(400);
    await window.getByTestId("files-commit").click();
    await window.waitForTimeout(500);
    await window.getByTestId("files-commit-message").fill(`Surface "no active commit point" state in Work panel

Adds a noCommitPointHint memo to PlanPane that detects the state
"batch has at least one human_check/done item but no live commit
points remaining" and renders an inline nudge above the bottom bar
(data-testid="plan-no-commit-point-hint"). Derived purely on the
client from batchWork + commitPoints state; no new store/IPC.

Closes the dogfood-loop gap where propose_commit silently no-op'd
without giving the outer user any Work-panel signal that a commit
was wanted but blocked.

.context/agent-model.md updated with the new signal in the same
commit.

Done by the inner agent during a /self-ralph dogfood run; outer
agent approved via newde's Files commit dialog.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`);
    await window.screenshot({ path: resolve(outDir, "cv-approve-filled.png") });
    await window.getByTestId("files-commit-submit").click();
    probeLog("[cv-approve] submitted");
    await window.waitForTimeout(3_000);
    await window.screenshot({ path: resolve(outDir, "cv-approve-done.png") });
  } finally {
    await close();
  }
}

runProbe("dogfood-cv-approve", main, { wallMs: 60_000, silenceMs: 45_000 }).catch(() => process.exit(1));
