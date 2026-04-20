/**
 * Probe: closing a dirty tab surfaces the themed ConfirmDialog (not the
 * unstyled native `window.confirm`). Verifies the Cancel path keeps the
 * tab open and the Discard path actually closes it.
 */
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { launchNewde, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const { window, close } = await launchNewde(projectDir);
  try {
    await window.waitForTimeout(3_000);

    // Open the file tree and a file.
    await window.getByTestId("dock-tab-project").click();
    await window.waitForTimeout(300);
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

    // Click the × close button — expect the themed dialog, not a native one.
    await window.getByTestId(`center-tab-close-${tabId}`).click();
    await window.waitForTimeout(200);

    const dialog = window.getByTestId("confirm-dialog");
    await dialog.waitFor({ timeout: 2_000 });
    console.log("[probe] themed confirm dialog visible");

    // Cancel keeps the tab open.
    await window.getByTestId("confirm-dialog-cancel").click();
    await window.waitForTimeout(300);
    const tabStillOpen = await window.evaluate(
      (id) => !!document.querySelector(`[data-testid="center-tab-${id}"]`),
      tabId,
    );
    if (!tabStillOpen) {
      console.log("[probe] FAIL: Cancel closed the tab anyway");
      process.exit(2);
    }

    // Close again, then Discard — tab should go away.
    await window.getByTestId(`center-tab-close-${tabId}`).click();
    await window.waitForTimeout(200);
    await window.getByTestId("confirm-dialog-confirm").click();
    await window.waitForTimeout(400);
    const tabGone = await window.evaluate(
      (id) => !document.querySelector(`[data-testid="center-tab-${id}"]`),
      tabId,
    );
    if (!tabGone) {
      console.log("[probe] FAIL: Discard did not close the tab");
      process.exit(3);
    }
    console.log("[probe] OK: dirty-close dialog cancel keeps tab, confirm discards");
  } finally {
    await close();
  }
}

runProbe("probe-dirty-close-dialog", main).catch(() => process.exit(1));
