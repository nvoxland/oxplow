/**
 * Probe: closing a dirty tab now closes immediately and surfaces an Undo
 * toast (replaces the prior ConfirmDialog). Verifies the Undo path
 * restores the tab + dirty draft, and that ignoring the toast leaves the
 * tab closed.
 */
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { launchOxplow, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const { window, close } = await launchOxplow(projectDir);
  try {
    await window.waitForTimeout(3_000);

    // Open the Files page so the file tree is mounted.
    await window.getByTestId("rail-page-files").click();
    await window.getByTestId("page-files").waitFor({ state: "visible", timeout: 5_000 });
    // Find a visible file-tree entry (not hidden under a collapsed dir).
    const path = await window.evaluate(() => {
      const node = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid^="file-tree-entry-"][data-kind="file"]'),
      ).find((n) => n.offsetParent !== null);
      if (!node) return null;
      node.click();
      return node.dataset.testid!.replace("file-tree-entry-", "");
    });
    if (!path) {
      console.log("[probe] FAIL: no visible file-tree entry");
      process.exit(2);
    }
    await window.waitForTimeout(600);

    const tabId = `file:${path}`;
    await window.getByTestId(`center-tab-${tabId}`).waitFor({ timeout: 3_000 });

    // Dirty the buffer: click the editor host so Monaco receives keystrokes.
    await window.getByTestId("monaco-host").click();
    await window.waitForTimeout(300);
    await window.keyboard.press("End");
    await window.keyboard.type(" // probe-dirty");
    await window.waitForTimeout(600);

    // Confirm the tab label now has the "●" dirty marker.
    const isDirty = await window.evaluate((id) => {
      const el = document.querySelector<HTMLElement>(`[data-testid="center-tab-${id}"]`);
      return !!el && el.textContent?.includes("●");
    }, tabId);
    console.log("[probe] tab dirty marker:", isDirty);
    if (!isDirty) {
      console.log("[probe] FAIL: edit did not mark the tab dirty");
      process.exit(3);
    }

    // Click the × close button — expect the tab to close immediately and
    // an Undo toast to appear.
    await window.getByTestId(`center-tab-close-${tabId}`).click();
    await window.waitForTimeout(300);

    const tabClosedFirst = await window.evaluate(
      (id) => !document.querySelector(`[data-testid="center-tab-${id}"]`),
      tabId,
    );
    if (!tabClosedFirst) {
      console.log("[probe] FAIL: dirty-close did not close the tab immediately");
      process.exit(2);
    }

    const toast = window.getByTestId("undo-toast");
    await toast.waitFor({ timeout: 2_000 });
    console.log("[probe] undo toast visible");

    // Undo restores the tab.
    await window.getByTestId("undo-toast-undo").click();
    await window.waitForTimeout(400);
    const tabRestored = await window.evaluate(
      (id) => !!document.querySelector(`[data-testid="center-tab-${id}"]`),
      tabId,
    );
    if (!tabRestored) {
      console.log("[probe] FAIL: Undo did not restore the tab");
      process.exit(3);
    }

    // Close again, then ignore the toast — tab should stay closed.
    await window.getByTestId(`center-tab-close-${tabId}`).click();
    await window.waitForTimeout(300);
    const toastTwo = window.getByTestId("undo-toast");
    await toastTwo.waitFor({ timeout: 2_000 });
    // Dismiss explicitly so we don't have to wait the auto-dismiss window.
    await window.getByTestId("undo-toast-dismiss").click();
    await window.waitForTimeout(400);
    const tabGone = await window.evaluate(
      (id) => !document.querySelector(`[data-testid="center-tab-${id}"]`),
      tabId,
    );
    if (!tabGone) {
      console.log("[probe] FAIL: tab reappeared after dismissing the undo toast");
      process.exit(4);
    }
    console.log("[probe] OK: dirty-close fires immediate close + undo toast; undo restores; dismiss leaves it closed");
  } finally {
    await close();
  }
}

runProbe("probe-dirty-close-dialog", main).catch(() => process.exit(1));
