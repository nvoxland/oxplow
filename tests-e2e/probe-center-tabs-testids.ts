/**
 * Probe: CenterTabs exposes `center-tab-<id>` and `center-tab-close-<id>`
 * testids. Opens a file, verifies the tab has a testid, then clicks the
 * close-tab button via its testid and verifies the tab disappears.
 */
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchOxplow, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchOxplow(projectDir);
  try {
    await window.waitForTimeout(3_000);

    // Open the Files page from the rail HUD.
    await window.getByTestId("rail-page-files").click();
    await window.getByTestId("page-files").waitFor({ state: "visible", timeout: 5_000 });

    // Click the first visible file in the tree.
    const firstFile = window.locator('[data-testid^="file-tree-entry-"][data-kind="file"]:visible').first();
    await firstFile.waitFor({ timeout: 5_000 });
    const filePath = await firstFile.getAttribute("data-testid");
    if (!filePath) {
      console.log("[probe] FAIL: first file has no data-testid");
      process.exit(2);
    }
    const relPath = filePath.replace("file-tree-entry-", "");
    console.log("[probe] opening file:", relPath);
    await firstFile.click();
    await window.waitForTimeout(600);

    // Tab ids are `file:<path>` (the "agent" tab is not closable).
    const tabId = `file:${relPath}`;
    const tab = window.getByTestId(`center-tab-${tabId}`);
    await tab.waitFor({ timeout: 3_000 });
    await window.screenshot({ path: resolve(outDir, "center-tabs-01-open.png") });

    // Close the tab via its testid.
    const closeBtn = window.getByTestId(`center-tab-close-${tabId}`);
    await closeBtn.waitFor({ timeout: 2_000 });
    await closeBtn.click();
    await window.waitForTimeout(400);

    const stillOpen = await window.evaluate((t) => {
      return !!document.querySelector(`[data-testid="center-tab-${t}"]`);
    }, tabId);
    if (stillOpen) {
      console.log("[probe] FAIL: tab still present after close click");
      process.exit(3);
    }
    await window.screenshot({ path: resolve(outDir, "center-tabs-02-closed.png") });
    console.log("[probe] OK: center-tab and center-tab-close testids reachable; close click works");
  } finally {
    await close();
  }
}

runProbe("probe-center-tabs-testids", main).catch(() => process.exit(1));
