// The rail's groups in a real engine (macOS's window is WebKit): dragging a
// panel by its handle shows the groups' dividers and the Remove zone, and
// a drop on a divider moves the panel there. Every panel's handle drags.
import type { Page } from "@playwright/test";

import { expect, test } from "../../support/fixtures.js";
import { load } from "../../support/ui.js";

async function sections(page: Page): Promise<string[]> {
  return page
    .locator('[data-testid^="rail-section-ext:"]')
    .evaluateAll((els) => els.map((el) => el.getAttribute("data-testid")!.slice("rail-section-".length)));
}

async function dragBy(page: Page, id: string, to: () => Promise<{ x: number; y: number }>) {
  const handle = page.getByTestId(`rail-section-drag-${id}`);
  const box = (await handle.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 12, { steps: 4 });
  const target = await to();
  await page.mouse.move(target.x, target.y, { steps: 6 });
}

test("every panel drags by its handle: the dividers show, and a drop on one moves it there", async ({ page }) => {
  await load(page);
  await expect(page.getByTestId("rail-section-ext:oxplow/work")).toBeVisible();
  const ids = await sections(page);
  expect(ids.length).toBeGreaterThan(1);
  for (const id of ids) {
    await dragBy(page, id, async () => {
      await expect(page.getByTestId("rail-group-divider-project")).toBeVisible();
      await expect(page.getByTestId("rail-remove-zone")).toBeVisible();
      const box = (await page.getByTestId("rail-group-divider-project").boundingBox())!;
      return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
    });
    await page.mouse.up();
    // The rail is the window's, not a tab's: no hidden copy to count.
    const rail = page.getByTestId("rail-hud");
    await expect(rail.getByTestId("rail-group-divider-project")).toHaveCount(0);
  }
  // Onto "This thread": the panel leads the rail, with the thread's icon.
  const moved = ids.at(-1)!;
  await dragBy(page, moved, async () => {
    const box = (await page.getByTestId("rail-group-divider-thread").boundingBox())!;
    return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  });
  await page.mouse.up();
  await expect.poll(() => sections(page).then((s) => s[0])).toBe(moved);
  await expect(page.getByTestId(`rail-section-shown-in-${moved}`)).toHaveAttribute("title", "Only in this thread");
});
