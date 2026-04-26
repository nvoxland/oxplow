/**
 * Probe: delete a work item via the row kebab → menu → Delete.
 *
 * Right-click + ConfirmDialog were retired by the IA cleanup; per-row
 * actions now live behind the always-visible kebab `⋯` button and
 * destructive actions fire immediately + an Undo toast.
 *
 * Verifies:
 *   (a) clicking the row kebab opens the context menu
 *   (b) clicking Delete fires immediately and removes the row (Undo
 *       toast appears but we ignore it so the dismiss path is exercised)
 *   (c) removal persists across a reload
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

    const planNewBtn = window.getByTestId("plan-new-task");
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

    // Find the work-item id for the row we just created so we can target
    // its always-visible kebab.
    const itemId = await window.evaluate((t) => {
      const row = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="work-item-row-"]'))
        .find((el) => el.getAttribute("title")?.startsWith(t));
      if (!row) return null;
      // testid format: work-item-row-<id>
      return row.dataset.testid?.replace(/^work-item-row-/, "") ?? null;
    }, title);
    if (!itemId) {
      console.log("[probe] FAIL: row not found");
      process.exit(3);
    }

    await window.getByTestId(`work-item-row-kebab-${itemId}`).click();
    await window.waitForTimeout(200);
    await window.screenshot({ path: resolve(outDir, "delete-ctx-02-menu.png") });

    const deleteBtn = window.getByTestId("menu-item-workitem.delete");
    await deleteBtn.waitFor({ timeout: 2_000 });
    await deleteBtn.click();
    // Delete now fires immediately and shows an Undo toast — no confirm
    // dialog. We ignore the toast so the dismiss/auto-expire path covers
    // the "stayed deleted" check.
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
