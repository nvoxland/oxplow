// Post-dogfood verification: open History, Snapshots, and Files-filter
// panes once each and confirm they show sensible content. Per the
// /self-ralph rule that mandates check-ins on these surfaces every
// pass.
//
// Not a strict assert probe — it captures screenshots and dumps
// short summaries so the outer agent can spot rot.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { launchOxplow, probeLog, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchOxplow(projectDir);
  try {
    await window.waitForTimeout(3_000);
    probeLog("[verify] launched");

    // History tab
    const dockTabs = await window.evaluate(() => {
      return Array.from(document.querySelectorAll("[data-testid^='dock-tab-']"))
        .map((el) => el.getAttribute("data-testid"));
    });
    probeLog(`[verify] dock tabs found: ${dockTabs.join(",")}`);

    const historyTab = window.locator("[data-testid='dock-tab-history']");
    if (await historyTab.count() > 0) {
      await historyTab.click();
      await window.waitForTimeout(1_500);
      await window.screenshot({ path: resolve(outDir, "verify-history.png") });
      const historyText = await window.evaluate(() => {
        const panel = document.querySelector("[data-testid='dock-panel-history']");
        return panel ? (panel as HTMLElement).innerText.slice(0, 800) : "(no history panel)";
      });
      writeFileSync(resolve(outDir, "verify-history.txt"), historyText);
      probeLog(`[verify] history first 200 chars: ${historyText.slice(0, 200).replace(/\n/g, " ⏎ ")}`);
    } else {
      probeLog("[verify] NO dock-tab-history found — finding for the log");
    }

    // Snapshots tab
    const snapsTab = window.locator("[data-testid='dock-tab-snapshots']");
    if (await snapsTab.count() > 0) {
      await snapsTab.click();
      await window.waitForTimeout(1_500);
      await window.screenshot({ path: resolve(outDir, "verify-snapshots.png") });
      const snapsText = await window.evaluate(() => {
        const panel = document.querySelector("[data-testid='dock-panel-snapshots']");
        return panel ? (panel as HTMLElement).innerText.slice(0, 800) : "(no snapshots panel)";
      });
      writeFileSync(resolve(outDir, "verify-snapshots.txt"), snapsText);
      probeLog(`[verify] snapshots first 200 chars: ${snapsText.slice(0, 200).replace(/\n/g, " ⏎ ")}`);
    } else {
      probeLog("[verify] NO dock-tab-snapshots found — finding for the log");
    }

    // Files panel + filter modes
    await window.locator("[data-testid='dock-tab-project']").click();
    await window.waitForTimeout(800);
    const filters = await window.evaluate(() => {
      return Array.from(document.querySelectorAll("button, [role='tab']"))
        .map((el) => (el as HTMLElement).innerText?.trim())
        .filter((t) => t && /uncommitted|branch|upstream|all files|changed/i.test(t));
    });
    probeLog(`[verify] file-filter-like buttons: ${JSON.stringify(filters)}`);
    await window.screenshot({ path: resolve(outDir, "verify-files-default.png") });

    // Open the filter popover via the canonical testid (added in 2e097c7)
    const filterToggle = window.locator("[data-testid='files-filter-toggle']");
    if (await filterToggle.count() > 0) {
      await filterToggle.click();
      await window.waitForTimeout(400);
      const opts = await window.evaluate(() => {
        return Array.from(document.querySelectorAll("[data-testid^='files-filter-option-']"))
          .map((el) => ({
            value: el.getAttribute("data-testid")?.replace("files-filter-option-", ""),
            label: (el as HTMLElement).innerText?.trim().slice(0, 60),
            disabled: (el as HTMLButtonElement).disabled,
          }));
      });
      probeLog(`[verify] filter options: ${JSON.stringify(opts)}`);
      // Click "uncommitted"
      const uncommitted = window.locator("[data-testid='files-filter-option-uncommitted']");
      if (await uncommitted.count() > 0) {
        await uncommitted.click();
        await window.waitForTimeout(500);
        await window.screenshot({ path: resolve(outDir, "verify-files-uncommitted.png") });
      }
    } else {
      probeLog("[verify] files-filter-toggle testid not found");
    }
  } finally {
    await close();
  }
}

runProbe("verify-history-snapshots-files", main, { wallMs: 90_000, silenceMs: 30_000 }).catch(() => process.exit(1));
