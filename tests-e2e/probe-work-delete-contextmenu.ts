/**
 * Probe: delete a work item via right-click → context menu → Delete.
 *
 * Verifies:
 *   (a) right-click on a row opens the context menu
 *   (b) clicking Delete + accepting the confirm dialog removes the row
 *   (c) removal persists across a reload
 */
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchNewde, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchNewde(projectDir);
  try {
    await window.waitForTimeout(3_000);

    const planNewBtn = window.getByTestId("plan-new-work-item");
    await planNewBtn.waitFor({ timeout: 10_000 });

    const title = `delete-ctx-probe-${Date.now()}`;
    await planNewBtn.click();
    await window.getByTestId("work-item-title").fill(title);
    await window.getByTestId("work-item-save").click();
    await window.waitForTimeout(400);

    // Confirm the row exists.
    const rowBefore = await window.evaluate((t) => {
      const rows = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="work-item-row-"]'));
      return rows.some((el) => el.getAttribute("title")?.startsWith(t));
    }, title);
    if (!rowBefore) {
      console.log("[probe] FAIL: row not created");
      process.exit(2);
    }


    await window.screenshot({ path: resolve(outDir, "delete-ctx-01-before.png") });

    // Dispatch a native `contextmenu` event on the row; React's onContextMenu
    // handler is what opens the menu. Playwright's .click({button:"right"})
    // also works, but the row has overlapping `<select>` elements for
    // status/priority that can eat right-clicks — go through the row div
    // directly.
    const opened = await window.evaluate((t) => {
      const row = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="work-item-row-"]'))
        .find((el) => el.getAttribute("title")?.startsWith(t));
      if (!row) return { ok: false, reason: "row not found" };
      const rect = row.getBoundingClientRect();
      const ev = new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: rect.left + 20,
        clientY: rect.top + 10,
        button: 2,
      });
      row.dispatchEvent(ev);
      return { ok: true };
    }, title);
    if (!opened.ok) {
      console.log("[probe] FAIL: could not dispatch contextmenu:", opened.reason);
      process.exit(3);
    }
    await window.waitForTimeout(150);
    await window.screenshot({ path: resolve(outDir, "delete-ctx-02-menu.png") });

    const deleteBtn = window.getByTestId("menu-item-workitem.delete");
    await deleteBtn.waitFor({ timeout: 2_000 });
    await deleteBtn.click();

    // Confirm in the themed destructive dialog.
    const confirmBtn = window.getByTestId("confirm-dialog-confirm");
    await confirmBtn.waitFor({ timeout: 2_000 });
    await confirmBtn.click();

    await window.waitForTimeout(600);
    await window.screenshot({ path: resolve(outDir, "delete-ctx-03-after.png") });

    const rowAfter = await window.evaluate((t) => {
      const rows = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="work-item-row-"]'));
      return rows.some((el) => el.getAttribute("title")?.startsWith(t));
    }, title);
    if (rowAfter) {
      console.log("[probe] FAIL: row still present after delete");
      process.exit(5);
    }

    // Reload & verify persistence.
    await window.reload();
    await window.waitForTimeout(2_000);
    const rowAfterReload = await window.evaluate((t) => {
      const rows = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="work-item-row-"]'));
      return rows.some((el) => el.getAttribute("title")?.startsWith(t));
    }, title);
    if (rowAfterReload) {
      console.log("[probe] FAIL: row reappeared after reload");
      process.exit(6);
    }

    console.log("[probe] OK: work item deleted via right-click menu and stayed deleted across reload");
  } finally {
    await close();
  }
}

runProbe("probe-work-delete-contextmenu", main).catch(() => process.exit(1));
