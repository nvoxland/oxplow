/**
 * Probe: drag a work item from its current batch onto another batch's
 * chip in the BatchRail. Verifies Scenario 5.
 *
 * Flow:
 *   1. Capture which batch is initially active.
 *   2. Create a second batch ("drop-target-<ts>").
 *   3. Create a work item in the initial batch.
 *   4. Synthesize an HTML5 drag from the work-item row onto the new
 *      batch's chip. The chip only accepts drops with the
 *      WORK_ITEM_DRAG_MIME type on the DataTransfer.
 *   5. Click the new batch's chip and verify the work item is now
 *      rendered there.
 *
 * Uses the repo dir as the project. Work-item title is timestamped so
 * probe runs can't collide on orphan rows leaked by prior runs
 * (separately tracked as a harness-isolation todo).
 */
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchNewde, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));
const WORK_ITEM_DRAG_MIME = "application/x-newde-work-item";

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const stamp = Date.now();
  const itemTitle = `drag-batch-probe-${stamp}`;
  const batchTitle = `drop-target-${stamp}`;

  const { window, close } = await launchNewde(projectDir);
  const fails: string[] = [];
  try {
    await window.waitForTimeout(3_000);

    // Make sure the Plan panel is visible (default, but be safe).
    await window.getByTestId("plan-new-work-item").waitFor({ timeout: 10_000 });

    // Create the target batch via the "+ New batch" button.
    await window.getByTestId("batch-rail-new").click();
    await window.waitForTimeout(150);
    const batchInput = window.locator('input[placeholder="Batch title"]');
    await batchInput.fill(batchTitle);
    await batchInput.press("Enter");
    await window.waitForTimeout(500);
    await window.screenshot({ path: resolve(outDir, "drag-batch-01-created-batch.png") });

    // Capture batch chip info via the `batch-chip-<id>` testid seam.
    const batchInfo = await window.evaluate(() => {
      const chips = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid^="batch-chip-"]'),
      );
      return chips.map((el) => ({
        id: el.dataset.testid?.replace("batch-chip-", "") ?? null,
        text: el.textContent?.trim().slice(0, 60),
      }));
    });
    console.log("[probe] batches after create:", JSON.stringify(batchInfo));
    const targetChip = batchInfo.find((b) => (b.text ?? "").includes(batchTitle));
    if (!targetChip?.id) {
      fails.push(`batch "${batchTitle}" not found after create`);
      return;
    }
    const targetBatchId = targetChip.id;

    // Create a work item in the currently-selected batch.
    await window.getByTestId("plan-new-work-item").click();
    await window.getByTestId("work-item-title").fill(itemTitle);
    await window.getByTestId("work-item-save").click();
    await window.waitForTimeout(500);

    const initialRow = window
      .locator('[data-testid^="work-item-row-"]', { hasText: itemTitle })
      .first();
    await initialRow.waitFor({ timeout: 5_000 });
    const rowTestid = await initialRow.getAttribute("data-testid");
    if (!rowTestid) { fails.push("row testid missing"); return; }
    const itemId = rowTestid.replace("work-item-row-", "");
    console.log("[probe] created item id:", itemId);

    // Synthesize the drag from the row → the target chip. Because drops are
    // gated on the WORK_ITEM_DRAG_MIME key existing in dataTransfer.types,
    // we build the DataTransfer by hand and seed the payload before firing
    // dragover/drop on the chip.
    const dragResult = await window.evaluate(
      ({ itemId, targetBatchId, mime }) => {
        const row = document.querySelector<HTMLElement>(
          `[data-testid="work-item-row-${itemId}"]`,
        );
        if (!row) return { ok: false, reason: "row not found" };

        // The chip testid is on the outer wrapper that owns the drop
        // handlers — exactly what we need. No DOM walking.
        const chip = document.querySelector<HTMLElement>(
          `[data-testid="batch-chip-${targetBatchId}"]`,
        );
        if (!chip) return { ok: false, reason: "target chip not found" };

        const dt = new DataTransfer();
        // Seed exactly what WorkGroupList's onDragStart sets.
        dt.setData("text/plain", itemId);
        dt.setData(mime, JSON.stringify({ itemId, fromBatchId: null }));

        const dragStart = new DragEvent("dragstart", {
          bubbles: true, cancelable: true, dataTransfer: dt,
        });
        row.dispatchEvent(dragStart);

        const dragOver = new DragEvent("dragover", {
          bubbles: true, cancelable: true, dataTransfer: dt,
        });
        chip.dispatchEvent(dragOver);

        const drop = new DragEvent("drop", {
          bubbles: true, cancelable: true, dataTransfer: dt,
        });
        chip.dispatchEvent(drop);

        const dragEnd = new DragEvent("dragend", {
          bubbles: true, cancelable: true, dataTransfer: dt,
        });
        row.dispatchEvent(dragEnd);

        return {
          ok: true,
          dropDefaultPrevented: drop.defaultPrevented,
          dragOverDefaultPrevented: dragOver.defaultPrevented,
        };
      },
      { itemId, targetBatchId, mime: WORK_ITEM_DRAG_MIME },
    );
    console.log("[probe] drag result:", JSON.stringify(dragResult));

    await window.waitForTimeout(600);
    await window.screenshot({ path: resolve(outDir, "drag-batch-02-after-drop.png") });

    // Click the target batch chip to select it and check the row is there.
    await window.getByTestId(`batch-chip-${targetBatchId}`).click();
    await window.waitForTimeout(400);
    await window.screenshot({ path: resolve(outDir, "drag-batch-03-target-selected.png") });

    const rowPresent = await window.evaluate((id) => {
      return !!document.querySelector(`[data-testid="work-item-row-${id}"]`);
    }, itemId);
    console.log("[probe] row present in target batch view?", rowPresent);
    if (!rowPresent) fails.push("work item row is not visible after switching to target batch");

    if (fails.length) {
      console.log("[probe] FAILURES:");
      for (const f of fails) console.log("  -", f);
      process.exit(2);
    }
    console.log("[probe] OK: work item moved to target batch via drag");
  } finally {
    await close();
  }
}

runProbe("probe-work-drag-batch", main).catch(() => process.exit(1));
