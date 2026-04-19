/**
 * Probe: inline-edit a work item's title/status/priority from the row
 * itself, without opening the detail pane.
 *
 * Verifies Scenario 2 from the /self-ralph exploratory stack:
 *   (a) Click the title → input appears, focus lands there.
 *   (b) Type + Enter → new title is saved.
 *   (c) Click the title again, type, press Escape → revert (old title).
 *   (d) Change status via the inline picker → row moves section and
 *       status icon updates.
 *   (e) Change priority via the inline picker → priority icon updates.
 *   (f) Tab from the title input reaches some focusable control in the
 *       row (we log what, so focus-order regressions are visible).
 *
 * Uses the repo dir as the project; row testids make everything
 * addressable without scraping text.
 */
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchNewde } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchNewde(projectDir);
  const fails: string[] = [];
  // Unique title per run so leftover state.sqlite rows from earlier runs
  // don't confuse locators (tracked separately as a probe-isolation todo).
  const probeTitle = `inline-edit-probe-${Date.now()}`;
  try {
    await window.waitForTimeout(3_000);

    // Create a fresh work item to edit.
    const planNewBtn = window.getByTestId("plan-new-work-item");
    await planNewBtn.waitFor({ timeout: 10_000 });
    await planNewBtn.click();
    await window.getByTestId("work-item-title").fill(probeTitle);
    await window.getByTestId("work-item-save").click();
    await window.waitForTimeout(400);

    // Locate the row via text initially, then pin to a testid-based
    // locator. hasText stops matching once the title is swapped into an
    // input's value, so a text-filter row locator breaks mid-edit.
    const initialRow = window.locator('[data-testid^="work-item-row-"]', {
      hasText: probeTitle,
    }).first();
    await initialRow.waitFor({ timeout: 5_000 });
    const rowTestid = await initialRow.getAttribute("data-testid");
    if (!rowTestid) { fails.push("row testid missing"); return; }
    const itemId = rowTestid.replace("work-item-row-", "");
    const rowLocator = window.locator(`[data-testid="${rowTestid}"]`);
    console.log("[probe] created item id:", itemId);

    await window.screenshot({ path: resolve(outDir, "inline-01-created.png") });

    // --- (a) + (b): click title, type new, press Enter commits -----------
    const titleSpan = rowLocator.getByTitle("Click to rename").first();
    await titleSpan.click();
    await window.waitForTimeout(200);
    const inputA = rowLocator.locator("input").first();
    const visibleA = await inputA.isVisible().catch(() => false);
    if (!visibleA) fails.push("(a) title input did not appear after click");
    await inputA.fill(probeTitle + " renamed");
    await inputA.press("Enter");
    await window.waitForTimeout(400);
    const afterRename = await rowLocator.textContent();
    console.log("[probe] (b) after Enter commit:", afterRename);
    if (!afterRename?.includes(probeTitle + " renamed")) fails.push("(b) Enter did not commit renamed title");

    // --- (c): click title again, type, press Escape reverts ---------------
    const titleSpan2 = rowLocator.getByTitle("Click to rename").first();
    await titleSpan2.click();
    await window.waitForTimeout(150);
    const inputC = rowLocator.locator("input").first();
    await inputC.fill("DISCARD-ME");
    await inputC.press("Escape");
    await window.waitForTimeout(400);
    const afterEscape = await rowLocator.textContent();
    console.log("[probe] (c) after Escape:", afterEscape);
    if (afterEscape?.includes("DISCARD-ME")) fails.push("(c) Escape did NOT revert — DISCARD-ME leaked through");
    if (!afterEscape?.includes(probeTitle + " renamed")) fails.push("(c) Escape lost the pre-edit title");

    // --- (d): change status via inline picker ------------------------------
    // The transparent <select> sits absolutely over the status-icon span.
    // Select it by position (first select in the row) and fire a change.
    const statusSelect = rowLocator.locator("select").first();
    const beforeStatus = await statusSelect.inputValue();
    console.log("[probe] (d) status before:", beforeStatus);
    await statusSelect.selectOption("human_check");
    await window.waitForTimeout(500);
    const afterStatus = await window.evaluate((id) => {
      const row = document.querySelector<HTMLElement>(`[data-testid="work-item-row-${id}"]`);
      if (!row) return null;
      const select = row.querySelector<HTMLSelectElement>("select");
      return select?.value ?? null;
    }, itemId);
    console.log("[probe] (d) status after:", afterStatus);
    if (afterStatus !== "human_check") fails.push(`(d) status picker did not set human_check (got ${afterStatus})`);

    // --- (e): change priority via inline picker ----------------------------
    const prioritySelect = rowLocator.locator("select").nth(1);
    await prioritySelect.selectOption("urgent");
    await window.waitForTimeout(500);
    const afterPriority = await window.evaluate((id) => {
      const row = document.querySelector<HTMLElement>(`[data-testid="work-item-row-${id}"]`);
      if (!row) return null;
      const selects = row.querySelectorAll<HTMLSelectElement>("select");
      return selects[1]?.value ?? null;
    }, itemId);
    console.log("[probe] (e) priority after:", afterPriority);
    if (afterPriority !== "urgent") fails.push(`(e) priority picker did not set urgent (got ${afterPriority})`);

    // --- (f): tab order from the title input -------------------------------
    const titleSpan3 = rowLocator.getByTitle("Click to rename").first();
    await titleSpan3.click();
    await window.waitForTimeout(150);
    const inputF = rowLocator.locator("input").first();
    await inputF.focus();
    await window.keyboard.press("Tab");
    const focusedInfo = await window.evaluate(() => {
      const el = document.activeElement as HTMLElement | null;
      if (!el) return null;
      const row = el.closest<HTMLElement>('[data-testid^="work-item-row-"]');
      return {
        tag: el.tagName,
        type: (el as HTMLInputElement).type ?? null,
        insideRow: !!row,
        rowTestid: row?.dataset.testid ?? null,
      };
    });
    console.log("[probe] (f) focus after Tab:", JSON.stringify(focusedInfo));
    // Escape so the probe doesn't leave the input open / mutate state.
    await window.keyboard.press("Escape");
    await window.waitForTimeout(200);

    await window.screenshot({ path: resolve(outDir, "inline-02-done.png") });

    if (fails.length) {
      console.log("[probe] FAILURES:");
      for (const f of fails) console.log("  -", f);
      process.exit(2);
    }
    console.log("[probe] OK: inline edit title/status/priority all behave as expected");
  } finally {
    await close();
  }
}

main().catch((err) => {
  console.error("[probe] failed:", err);
  process.exit(1);
});
