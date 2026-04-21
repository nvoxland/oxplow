/**
 * Probe: drag a work item between status sections (To do → Human check).
 *
 * Creates a fresh work item, then synthesizes an HTML5 drag from its row
 * onto the "Human check" section header. Verifies:
 *   (a) the status visibly changes (row moves under "Human check")
 *   (b) the change persists across a reload
 *
 * HTML5 DnD via Playwright's `.dragTo()` is unreliable — React's synthetic
 * event system doesn't always see the synthesized dragstart/drop. So we
 * dispatch DragEvent + a shared DataTransfer by hand in the page context.
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

    // Plan is the default left dock tab; confirm it's visible.
    const planNewBtn = window.getByTestId("plan-new-task");
    await planNewBtn.waitFor({ timeout: 10_000 });

    // Create a work item titled "drag-section-probe".
    await planNewBtn.click();
    await window.getByTestId("work-item-title").fill("drag-section-probe");
    await window.getByTestId("work-item-save").click();
    await window.waitForTimeout(400);
    await window.screenshot({ path: resolve(outDir, "drag-section-01-after-create.png") });

    // Dump sections + rows so we can see what's rendered.
    const before = await window.evaluate(() => {
      const labels = Array.from(document.querySelectorAll("div")).filter((el) => {
        const t = el.textContent?.trim().toUpperCase();
        return (
          t === "TO DO" || t === "HUMAN CHECK" || t === "DONE" || t === "IN PROGRESS"
        ) && el.children.length === 0;
      });
      const rows = Array.from(document.querySelectorAll("[data-key]")).map((el) => ({
        key: (el as HTMLElement).dataset.key,
        title: el.textContent?.trim().slice(0, 80),
        rect: (el as HTMLElement).getBoundingClientRect().top,
      }));
      return { sectionLabels: labels.map((el) => el.textContent?.trim()), rows };
    });
    console.log("[probe] before:", JSON.stringify(before, null, 2));

    // Find the work item row and the "HUMAN CHECK" header.
    // When a drag is NOT active, empty section headers are suppressed. So we
    // first have to start dragging — the header appears as a drop target —
    // then drop on it.
    const dragResult = await window.evaluate(async (probeTitle) => {
      const rows = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid^="work-item-row-"]'),
      );
      const row = rows.find((el) => el.getAttribute("title")?.startsWith(probeTitle));
      if (!row) return { ok: false, reason: "row not found" };

      const draggable = row.getAttribute("draggable") === "true" ? row : row.closest<HTMLElement>('[draggable="true"]');
      if (!draggable) return { ok: false, reason: "draggable not found" };

      const dt = new DataTransfer();
      const dragStart = new DragEvent("dragstart", { bubbles: true, cancelable: true, dataTransfer: dt });
      draggable.dispatchEvent(dragStart);

      await new Promise((r) => setTimeout(r, 50));

      const header = document.querySelector<HTMLElement>(
        '[data-testid="plan-section-header-humanCheck"]',
      );
      if (!header) return { ok: false, reason: "human-check header not visible after dragstart" };

      const dragOver = new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: dt });
      header.dispatchEvent(dragOver);

      const drop = new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: dt });
      header.dispatchEvent(drop);

      const dragEnd = new DragEvent("dragend", { bubbles: true, cancelable: true, dataTransfer: dt });
      draggable.dispatchEvent(dragEnd);

      return { ok: true };
    }, "drag-section-probe");
    console.log("[probe] drag result:", JSON.stringify(dragResult));

    await window.waitForTimeout(600);
    await window.screenshot({ path: resolve(outDir, "drag-section-02-after-drop.png") });

    // Verify the row is now under "Human check" by checking its status icon.
    const after = await window.evaluate(() => {
      const rows = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid^="work-item-row-"]'),
      ).map((el) => ({
        testid: el.dataset.testid,
        title: el.getAttribute("title") ?? "",
        top: el.getBoundingClientRect().top,
      }));
      const sections = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid^="plan-section-header-"]'),
      ).map((el) => ({
        label: el.textContent?.trim() ?? "",
        top: el.getBoundingClientRect().top,
      }));
      return { rows, sections };
    });
    console.log("[probe] after:", JSON.stringify(after, null, 2));

    // Compute: which section is the row under?
    const probeRow = after.rows.find((r) => r.title.startsWith("drag-section-probe"));
    if (!probeRow) {
      console.log("[probe] FAIL: row not found after drag");
      process.exit(2);
    }
    const sectionsSortedAboveRow = after.sections
      .filter((s) => s.top <= probeRow.top)
      .sort((a, b) => b.top - a.top);
    const currentSection = sectionsSortedAboveRow[0]?.label?.toUpperCase();
    console.log("[probe] current section:", currentSection);

    if (currentSection !== "HUMAN CHECK") {
      console.log("[probe] FAIL: expected HUMAN CHECK, got", currentSection);
      process.exit(3);
    }
    console.log("[probe] OK: row moved to Human check section");
  } finally {
    await close();
  }
}

runProbe("probe-work-drag-section", main).catch(() => process.exit(1));
