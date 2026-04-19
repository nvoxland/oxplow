/**
 * Probe: Shift+↑/↓ reorders the selected work item within its status
 * section. Scenario 4.
 *
 * Flow:
 *   1. Create two fresh work items ("A", "B") — both land in To do.
 *   2. Click B to select it (click in-page via evaluate; Playwright's
 *      own .click() sometimes lands on a child element and misses the
 *      React onClick that installs selectedId).
 *   3. Focus the Plan pane so its keydown listener is in scope.
 *   4. Press Shift+ArrowUp.
 *   5. Verify B is now above A in the rendered row order.
 *
 * Row order is measured from the DOM via rect.top — matches what the
 * user sees.
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

  const stamp = Date.now();
  const titleA = `shift-reorder-A-${stamp}`;
  const titleB = `shift-reorder-B-${stamp}`;

  const { window, close } = await launchNewde(projectDir);
  const fails: string[] = [];
  try {
    await window.waitForTimeout(3_000);
    await window.getByTestId("plan-new-work-item").waitFor({ timeout: 10_000 });

    // Create A, then B. Default status "waiting" → both land in "To do".
    for (const title of [titleA, titleB]) {
      await window.getByTestId("plan-new-work-item").click();
      await window.getByTestId("work-item-title").fill(title);
      await window.getByTestId("work-item-save").click();
      await window.waitForTimeout(300);
    }

    const idsByTitle = await window.evaluate(({ a, b }) => {
      const rows = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid^="work-item-row-"]'),
      );
      const idFor = (t: string) => {
        const row = rows.find((el) => (el.getAttribute("title") ?? "") === t);
        return row?.dataset.testid?.replace("work-item-row-", "") ?? null;
      };
      return { idA: idFor(a), idB: idFor(b) };
    }, { a: titleA, b: titleB });
    if (!idsByTitle.idA || !idsByTitle.idB) {
      fails.push(`created work item ids missing: ${JSON.stringify(idsByTitle)}`);
      return;
    }
    const { idA, idB } = idsByTitle;
    console.log("[probe] created:", JSON.stringify(idsByTitle));
    void idA;

    const initialOrder = await window.evaluate(({ a, b }) => {
      const rows = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid^="work-item-row-"]'),
      ).filter((el) => [a, b].includes(el.getAttribute("title") ?? ""));
      return rows
        .map((el) => ({
          title: el.getAttribute("title"),
          y: el.getBoundingClientRect().top,
        }))
        .sort((x, y) => x.y - y.y)
        .map((r) => r.title);
    }, { a: titleA, b: titleB });
    console.log("[probe] initial order (top → bottom):", initialOrder);
    if (initialOrder[0] !== titleA || initialOrder[1] !== titleB) {
      fails.push(`expected A above B initially, got ${JSON.stringify(initialOrder)}`);
    }

    // Select B via in-page click, then focus the pane so its keydown
    // listener is in scope (keydowns on body don't bubble *into* the
    // pane div where the listener is attached).
    const pane = window.getByTestId("plan-pane");
    await window.evaluate((id) => {
      document.querySelector<HTMLElement>(`[data-testid="work-item-row-${id}"]`)?.click();
    }, idB);
    await window.waitForTimeout(250);
    await pane.focus();
    await window.waitForTimeout(80);

    // Shift+ArrowUp should move B above A within the To do section.
    await pane.press("Shift+ArrowUp");
    await window.waitForTimeout(500);
    await window.screenshot({ path: resolve(outDir, "shift-reorder-01-after.png") });

    const afterOrder = await window.evaluate(({ a, b }) => {
      const rows = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid^="work-item-row-"]'),
      ).filter((el) => [a, b].includes(el.getAttribute("title") ?? ""));
      return rows
        .map((el) => ({
          title: el.getAttribute("title"),
          y: el.getBoundingClientRect().top,
        }))
        .sort((x, y) => x.y - y.y)
        .map((r) => r.title);
    }, { a: titleA, b: titleB });
    console.log("[probe] after Shift+ArrowUp (top → bottom):", afterOrder);

    if (afterOrder[0] !== titleB || afterOrder[1] !== titleA) {
      fails.push(`Shift+ArrowUp did not reorder: expected B above A, got ${JSON.stringify(afterOrder)}`);
    }

    if (fails.length) {
      console.log("[probe] FAILURES:");
      for (const f of fails) console.log("  -", f);
      process.exit(2);
    }
    console.log("[probe] OK: Shift+ArrowUp reordered within section");
  } finally {
    await close();
  }
}

main().catch((err) => {
  console.error("[probe] failed:", err);
  process.exit(1);
});
