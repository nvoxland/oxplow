/**
 * Probe: drive every newly-populated palette entry through Cmd+K.
 *
 * Scenario-level workflow (rotation pass 3): palette populated in
 * 19ef17a + 55a4e1d. Verifies each command actually activates its
 * target UI affordance, not just that it appears in the list.
 *
 * Steps:
 *   1. Cmd+K → "history" → Enter → Git history page becomes active.
 *   2. Cmd+K → "snapshot" → Enter → Local history page becomes active.
 *   3. Cmd+K → "thread new" → Enter → inline thread-create input appears.
 */
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

    const isMac = process.platform === "darwin";
    const modKey = isMac ? "Meta" : "Control";

    async function runPaletteEntry(query: string, step: string) {
      await window.keyboard.press(`${modKey}+k`);
      await window.getByTestId("command-palette-input").waitFor({ timeout: 3_000 });
      await window.getByTestId("command-palette-input").fill(query);
      await window.waitForTimeout(150);
      await window.keyboard.press("Enter");
      await window.waitForTimeout(400);
      await window.screenshot({ path: resolve(outDir, `palette-wf-${step}.png`) });
    }

    // Step 1: history.open — opens GitHistoryPage as a center tab
    await runPaletteEntry("history", "01-history");
    const historyActive = await window.evaluate(() => {
      const page = document.querySelector('[data-testid="page-git-history"]') as HTMLElement | null;
      return !!page && page.offsetParent !== null;
    });
    probeLog(`[probe] git-history page active: ${historyActive}`);
    if (!historyActive) {
      probeLog("[probe] FAIL: history.open did not surface page-git-history");
      process.exit(2);
    }

    // Step 2: snapshots.open — opens LocalHistoryPage as a center tab
    await runPaletteEntry("snapshots", "02-snapshots");
    const snapshotsActive = await window.evaluate(() => {
      const page = document.querySelector('[data-testid="page-local-history"]') as HTMLElement | null;
      return !!page && page.offsetParent !== null;
    });
    probeLog(`[probe] local-history page active: ${snapshotsActive}`);
    if (!snapshotsActive) {
      probeLog("[probe] FAIL: snapshots.open did not surface page-local-history");
      process.exit(3);
    }

    // Step 3: thread.new. Queries like "thread new" (label-first) used
    // to fail against "Work › New Thread" because the fuzzy match is
    // order-sensitive and the group was prefixed. The alt search key
    // ("label group") should now catch it.
    await runPaletteEntry("thread new", "03-thread-new");
    // The ThreadRail surfaces the create input either inline or via a dialog.
    // We check for a focused text input that wasn't there before; the
    // ThreadRail create-request counter routes to an input.
    await window.waitForTimeout(400);
    const threadCreateVisible = await window.evaluate(() => {
      const input = document.querySelector('[data-testid="thread-rail-create-input"]');
      return !!input && (input as HTMLElement).offsetParent !== null;
    });
    probeLog(`[probe] thread create input visible: ${threadCreateVisible}`);
    if (!threadCreateVisible) {
      probeLog("[probe] FAIL: thread.new did not surface the create input");
      process.exit(4);
    }

    probeLog("[probe] OK: palette-workflow drove 3 entries to their target UIs");
  } finally {
    await close();
  }
}

runProbe("probe-palette-workflow", main).catch(() => process.exit(1));
