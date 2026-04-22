/**
 * Probe: thread lifecycle — create, rename via right-click, promote to writer,
 * complete.
 *
 * Scenario 7 from /self-ralph todo. Watches for stale UI when active thread
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

    // --- Step 1: create a new thread ---
    const unique = Date.now();
    const thread1Title = `probe-thread-1-${unique}`;
    const thread1Renamed = `probe-thread-1-renamed-${unique}`;
    const thread2Title = `probe-thread-2-${unique}`;

    await window.getByTestId("thread-rail-new").click();
    await window.getByTestId("thread-rail-create-input").fill(thread1Title);
    await window.getByTestId("thread-rail-create-submit").click();
    await window.waitForTimeout(500);

    const thread1Id = await window.evaluate((t) => {
      const chip = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="thread-chip-"]'))
        .find((el) => el.textContent?.includes(t));
      return chip?.dataset.testid?.replace("thread-chip-", "") ?? null;
    }, thread1Title);
    if (!thread1Id) {
      console.log("[probe] FAIL: thread 1 chip not found after create");
      process.exit(2);
    }
    console.log("[probe] created thread1:", thread1Id);
    await window.screenshot({ path: resolve(outDir, "thread-01-created.png") });

    // --- Step 2: rename via right-click context menu ---
    const chip1 = window.getByTestId(`thread-chip-${thread1Id}`);
    await chip1.click({ button: "right" });
    await window.waitForTimeout(200);
    // The shared ContextMenu component renders MenuList buttons with the
    // label text. Rename… is the first item.
    const renameBtn = window.locator('button:has-text("Rename…")').first();
    await renameBtn.click();
    await window.waitForTimeout(200);

    const renameInput = window.getByTestId(`thread-chip-rename-input-${thread1Id}`);
    await renameInput.waitFor({ timeout: 2_000 });
    await renameInput.fill(thread1Renamed);
    await renameInput.press("Enter");
    await window.waitForTimeout(400);

    const renamed = await window.evaluate((id) => {
      const chip = document.querySelector<HTMLElement>(`[data-testid="thread-chip-${id}"]`);
      return chip?.textContent ?? null;
    }, thread1Id);
    if (!renamed?.includes(thread1Renamed)) {
      console.log("[probe] FAIL: rename did not stick. chip contents:", renamed);
      process.exit(3);
    }
    console.log("[probe] renamed ok");

    // --- Step 3: create a second thread so thread1 has someone to hand off to ---
    await window.getByTestId("thread-rail-new").click();
    await window.getByTestId("thread-rail-create-input").fill(thread2Title);
    await window.getByTestId("thread-rail-create-submit").click();
    await window.waitForTimeout(500);

    const thread2Id = await window.evaluate((t) => {
      const chip = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="thread-chip-"]'))
        .find((el) => el.textContent?.includes(t));
      return chip?.dataset.testid?.replace("thread-chip-", "") ?? null;
    }, thread2Title);
    if (!thread2Id) {
      console.log("[probe] FAIL: thread 2 not created");
      process.exit(4);
    }
    console.log("[probe] created thread2:", thread2Id);

    // --- Step 4: capture which chip currently has the writer badge ---
    const writerBefore = await window.evaluate(() => {
      const chips = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="thread-chip-"]'));
      return chips.filter((c) => c.textContent?.includes("✎"))
        .map((c) => c.dataset.testid);
    });
    console.log("[probe] writer chips before promote:", writerBefore);

    // --- Step 5: promote thread2 to writer via hover card ---
    const chip2 = window.getByTestId(`thread-chip-${thread2Id}`);
    await chip2.hover();
    await window.waitForTimeout(400); // 250ms schedule + a little slack

    const promoteBtn = window.getByTestId(`thread-chip-promote-${thread2Id}`);
    await promoteBtn.waitFor({ timeout: 2_000 });
    await promoteBtn.click();
    await window.waitForTimeout(500);

    const writerAfter = await window.evaluate(() => {
      const chips = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="thread-chip-"]'));
      return chips.filter((c) => c.textContent?.includes("✎"))
        .map((c) => c.dataset.testid);
    });
    console.log("[probe] writer chips after promote:", writerAfter);
    await window.screenshot({ path: resolve(outDir, "thread-02-after-promote.png") });

    if (!writerAfter.includes(`thread-chip-${thread2Id}`)) {
      console.log("[probe] FAIL: thread2 did not receive writer badge after promote");
      process.exit(5);
    }
    if (writerAfter.length !== 1) {
      console.log("[probe] FAIL: expected exactly one writer, got", writerAfter);
      process.exit(6);
    }

    // --- Step 6: complete thread2 (hand back to thread1, which is queued) ---
    await window.getByTestId(`thread-chip-${thread2Id}`).hover();
    await window.waitForTimeout(400);
    const completeBtn = window.getByTestId(`thread-chip-complete-${thread2Id}`);
    await completeBtn.waitFor({ timeout: 2_000 });
    await completeBtn.click();
    await window.waitForTimeout(600);
    await window.screenshot({ path: resolve(outDir, "thread-03-after-complete.png") });

    // After completion, thread2 should move off the active rail into the
    // "… N done ▾" overflow. The writer should be thread1 again.
    const finalState = await window.evaluate((ids) => {
      const chips = Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="thread-chip-"]'));
      const visible = chips.filter((c) => c.offsetParent !== null).map((c) => c.dataset.testid);
      const writer = chips.filter((c) => c.textContent?.includes("✎")).map((c) => c.dataset.testid);
      const overflowBtn = Array.from(document.querySelectorAll<HTMLElement>("button"))
        .find((b) => /done ▾/.test(b.textContent ?? ""));
      return { visible, writer, overflowBtnText: overflowBtn?.textContent ?? null, ids };
    }, { thread1Id, thread2Id });
    console.log("[probe] final state:", JSON.stringify(finalState, null, 2));

    // The writer role should transfer to SOME other queued thread — but which
    // one depends on sort_index across all threads in the project, not just
    // the ones this probe created (other tests/sessions may leave queued
    // threads around). Assert (a) writer is no longer thread2, (b) there's
    // still exactly one writer, (c) thread2 is no longer in the visible rail
    // (it moved to the "… N done ▾" overflow).
    if (finalState.writer.includes(`thread-chip-${thread2Id}`)) {
      console.log("[probe] FAIL: completed thread2 still shows writer badge");
      process.exit(7);
    }
    if (finalState.writer.length !== 1) {
      console.log("[probe] FAIL: expected exactly one writer after complete, got", finalState.writer);
      process.exit(8);
    }
    if (finalState.visible.includes(`thread-chip-${thread2Id}`)) {
      console.log("[probe] FAIL: completed thread2 still visible on rail");
      process.exit(9);
    }

    console.log("[probe] OK: thread lifecycle — create, rename, promote, complete — all healthy");
  } finally {
    await close();
  }
}

runProbe("probe-thread-lifecycle", main).catch(() => process.exit(1));
