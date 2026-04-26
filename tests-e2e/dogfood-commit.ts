// Final step of the dogfood flow: the inner agent's propose_commit
// no-op'd (no active commit point existed), so the user commits via
// the Files panel instead. Exercises the files-commit flow added in
// commit 55a4e1d.
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
    probeLog("[commit] oxplow launched");

    // Open Files page.
    await window.getByTestId("rail-page-files").click();
    await window.getByTestId("page-files").waitFor({ state: "visible", timeout: 5_000 });
    await window.waitForTimeout(400);

    // Click the Commit (N) button.
    const commitBtn = window.getByTestId("files-commit");
    await commitBtn.waitFor({ timeout: 5_000 });
    await commitBtn.click();
    await window.waitForTimeout(500);
    await window.screenshot({ path: resolve(outDir, "commit-01-dialog.png") });
    probeLog("[commit] commit dialog opened");

    // Fill commit message and submit.
    const msg = window.getByTestId("files-commit-message");
    await msg.waitFor({ timeout: 3_000 });
    await msg.fill(`Expand work-item right-click menu beyond Delete

Adds Rename… / Change status… / Change priority… entries alongside
Delete, mirroring the inline click and s/p keyboard shortcuts so
keyboard-first users don't have to hover. Follows the ThreadRail
context-menu expansion in 7cc3302.

.context/usability.md updated with the new menu-item-workitem.*
testids in the same change.

Done by the inner agent during a /self-ralph dogfood run; outer
agent approved the commit through the UI.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`);
    await window.screenshot({ path: resolve(outDir, "commit-02-filled.png") });

    const submit = window.getByTestId("files-commit-submit");
    await submit.click();
    probeLog("[commit] submitted");
    await window.waitForTimeout(3_000);
    await window.screenshot({ path: resolve(outDir, "commit-03-after.png") });
    probeLog("[commit] done");
  } finally {
    await close();
  }
}

runProbe("dogfood-commit", main, { wallMs: 60_000, silenceMs: 45_000 }).catch(() => process.exit(1));
