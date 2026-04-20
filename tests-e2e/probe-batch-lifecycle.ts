/**
 * Probe: batch lifecycle — create, rename via right-click, promote to writer,
 * complete.
 *
 * Scenario 7 from /self-ralph todo. Watches for stale UI when active batch
 * changes mid-flow: after promoting B2, the writer badge (✎) should move to
 * B2 and off B1 immediately without a reload.
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

    // --- Step 1: create a new batch ---
    const unique = Date.now();
    const batch1Title = `probe-batch-1-${unique}`;
    const batch1Renamed = `probe-batch-1-renamed-${unique}`;
    const batch2Title = `probe-batch-2-${unique}`;

    await window.getByTestId("batch-rail-new").click();
    await window.getByTestId("batch-rail-create-input").fill(batch1Title);
    await window.getByTestId("batch-rail-create-submit").click();
    await window.waitForTimeout(500);

    const batch1Id = await window.evaluate((t) => {
      const chip = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="batch-chip-"]'))
        .find((el) => el.textContent?.includes(t));
      return chip?.dataset.testid?.replace("batch-chip-", "") ?? null;
    }, batch1Title);
    if (!batch1Id) {
      console.log("[probe] FAIL: batch 1 chip not found after create");
      process.exit(2);
    }
    console.log("[probe] created batch1:", batch1Id);
    await window.screenshot({ path: resolve(outDir, "batch-01-created.png") });

    // --- Step 2: rename via right-click context menu ---
    const chip1 = window.getByTestId(`batch-chip-${batch1Id}`);
    await chip1.click({ button: "right" });
    await window.waitForTimeout(200);
    // The shared ContextMenu component renders MenuList buttons with the
    // label text. Rename… is the first item.
    const renameBtn = window.locator('button:has-text("Rename…")').first();
    await renameBtn.click();
    await window.waitForTimeout(200);

    const renameInput = window.getByTestId(`batch-chip-rename-input-${batch1Id}`);
    await renameInput.waitFor({ timeout: 2_000 });
    await renameInput.fill(batch1Renamed);
    await renameInput.press("Enter");
    await window.waitForTimeout(400);

    const renamed = await window.evaluate((id) => {
      const chip = document.querySelector<HTMLElement>(`[data-testid="batch-chip-${id}"]`);
      return chip?.textContent ?? null;
    }, batch1Id);
    if (!renamed?.includes(batch1Renamed)) {
      console.log("[probe] FAIL: rename did not stick. chip contents:", renamed);
      process.exit(3);
    }
    console.log("[probe] renamed ok");

    // --- Step 3: create a second batch so batch1 has someone to hand off to ---
    await window.getByTestId("batch-rail-new").click();
    await window.getByTestId("batch-rail-create-input").fill(batch2Title);
    await window.getByTestId("batch-rail-create-submit").click();
    await window.waitForTimeout(500);

    const batch2Id = await window.evaluate((t) => {
      const chip = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="batch-chip-"]'))
        .find((el) => el.textContent?.includes(t));
      return chip?.dataset.testid?.replace("batch-chip-", "") ?? null;
    }, batch2Title);
    if (!batch2Id) {
      console.log("[probe] FAIL: batch 2 not created");
      process.exit(4);
    }
    console.log("[probe] created batch2:", batch2Id);

    // --- Step 4: capture which chip currently has the writer badge ---
    const writerBefore = await window.evaluate(() => {
      const chips = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="batch-chip-"]'));
      return chips.filter((c) => c.textContent?.includes("✎"))
        .map((c) => c.dataset.testid);
    });
    console.log("[probe] writer chips before promote:", writerBefore);

    // --- Step 5: promote batch2 to writer via hover card ---
    const chip2 = window.getByTestId(`batch-chip-${batch2Id}`);
    await chip2.hover();
    await window.waitForTimeout(400); // 250ms schedule + a little slack

    const promoteBtn = window.getByTestId(`batch-chip-promote-${batch2Id}`);
    await promoteBtn.waitFor({ timeout: 2_000 });
    await promoteBtn.click();
    await window.waitForTimeout(500);

    const writerAfter = await window.evaluate(() => {
      const chips = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="batch-chip-"]'));
      return chips.filter((c) => c.textContent?.includes("✎"))
        .map((c) => c.dataset.testid);
    });
    console.log("[probe] writer chips after promote:", writerAfter);
    await window.screenshot({ path: resolve(outDir, "batch-02-after-promote.png") });

    if (!writerAfter.includes(`batch-chip-${batch2Id}`)) {
      console.log("[probe] FAIL: batch2 did not receive writer badge after promote");
      process.exit(5);
    }
    if (writerAfter.length !== 1) {
      console.log("[probe] FAIL: expected exactly one writer, got", writerAfter);
      process.exit(6);
    }

    // --- Step 6: complete batch2 (hand back to batch1, which is queued) ---
    await window.getByTestId(`batch-chip-${batch2Id}`).hover();
    await window.waitForTimeout(400);
    const completeBtn = window.getByTestId(`batch-chip-complete-${batch2Id}`);
    await completeBtn.waitFor({ timeout: 2_000 });
    await completeBtn.click();
    await window.waitForTimeout(600);
    await window.screenshot({ path: resolve(outDir, "batch-03-after-complete.png") });

    // After completion, batch2 should move off the active rail into the
    // "… N done ▾" overflow. The writer should be batch1 again.
    const finalState = await window.evaluate((ids) => {
      const chips = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="batch-chip-"]'));
      const visible = chips.filter((c) => c.offsetParent !== null).map((c) => c.dataset.testid);
      const writer = chips.filter((c) => c.textContent?.includes("✎")).map((c) => c.dataset.testid);
      const overflowBtn = Array.from(document.querySelectorAll<HTMLElement>("button"))
        .find((b) => /done ▾/.test(b.textContent ?? ""));
      return { visible, writer, overflowBtnText: overflowBtn?.textContent ?? null, ids };
    }, { batch1Id, batch2Id });
    console.log("[probe] final state:", JSON.stringify(finalState, null, 2));

    // The writer role should transfer to SOME other queued batch — but which
    // one depends on sort_index across all batches in the project, not just
    // the ones this probe created (other tests/sessions may leave queued
    // batches around). Assert (a) writer is no longer batch2, (b) there's
    // still exactly one writer, (c) batch2 is no longer in the visible rail
    // (it moved to the "… N done ▾" overflow).
    if (finalState.writer.includes(`batch-chip-${batch2Id}`)) {
      console.log("[probe] FAIL: completed batch2 still shows writer badge");
      process.exit(7);
    }
    if (finalState.writer.length !== 1) {
      console.log("[probe] FAIL: expected exactly one writer after complete, got", finalState.writer);
      process.exit(8);
    }
    if (finalState.visible.includes(`batch-chip-${batch2Id}`)) {
      console.log("[probe] FAIL: completed batch2 still visible on rail");
      process.exit(9);
    }

    console.log("[probe] OK: batch lifecycle — create, rename, promote, complete — all healthy");
  } finally {
    await close();
  }
}

runProbe("probe-batch-lifecycle", main).catch(() => process.exit(1));
