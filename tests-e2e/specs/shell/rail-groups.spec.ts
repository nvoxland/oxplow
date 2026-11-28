// @desktop-only: the rail's groups, drag and remove zone
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

test("a drag held near the rail's bottom scrolls it, to reach a drop target out of view", async ({ page, browserName }) => {
  // Playwright's WebKit drag sends dragover for its first moves only, then
  // the drop: it can't hold a drag near an edge. A real one does, every few
  // hundred ms, and the scrolling is the same code in both engines.
  test.skip(browserName === "webkit", "Playwright's WebKit drag can't hold near an edge");
  // Short enough that the rail scrolls.
  await page.setViewportSize({ width: 1200, height: 420 });
  await load(page);
  await expect(page.getByTestId("rail-section-ext:oxplow/work")).toBeVisible();
  const rail = page.getByTestId("rail-panels");
  expect(await rail.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
  const first = (await sections(page))[0]!;
  const box = (await rail.boundingBox())!;
  await dragBy(page, first, async () => ({ x: box.x + box.width / 2, y: box.y + box.height - 6 }));
  // Held there, the rail scrolls until the Remove zone is in view. It may
  // scroll on a little: the zone is measured once two reads agree.
  const zone = page.getByTestId("rail-remove-zone");
  await expect(zone).toBeInViewport();
  const scrolled: number[] = [];
  await expect
    .poll(async () => {
      scrolled.push(await rail.evaluate((el) => el.scrollTop));
      return scrolled.length > 1 && scrolled.at(-1) === scrolled.at(-2);
    })
    .toBe(true);
  // The pointer is still near the edge: a move over the zone drops there.
  const z = (await zone.boundingBox())!;
  await page.mouse.move(z.x + z.width / 2, z.y + z.height / 2, { steps: 3 });
  await page.mouse.up();
  await expect.poll(() => sections(page)).not.toContain(first);
});
