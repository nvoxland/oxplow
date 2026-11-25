// The narrow-screen check (`expectNoHorizontalOverflow`) must be able to fail:
// each case draws a box the check has to refuse, on a bare page with no daemon.
import type { Page } from "@playwright/test";
import { expect, test } from "@playwright/test";

import { expectNoHorizontalOverflow } from "../../support/ui.js";

/** A 400px-wide page with a shown tab body holding `body`, and `nav` above it. */
async function draw(page: Page, body: string, nav = "") {
  await page.setViewportSize({ width: 400, height: 600 });
  await page.setContent(
    `<body style="margin:0">${nav}<div data-testid="center-tab-body-x" style="width:300px;position:relative">${body}</div></body>`,
  );
}

test("a box past the shown tab's right edge fails", async ({ page }) => {
  await draw(page, `<div style="width:350px;height:20px">wide</div>`);
  await expect(expectNoHorizontalOverflow(page)).rejects.toThrow(/reaches 350 > 300/);
});

test("a box past a narrower clipping ancestor fails, though inside the tab", async ({ page }) => {
  await draw(page, `<div style="width:150px;overflow:hidden"><div style="width:250px;height:20px">cut</div></div>`);
  await expect(expectNoHorizontalOverflow(page)).rejects.toThrow(/reaches 250 > 150/);
});

test("the page's nav bar past the window's edge fails", async ({ page }) => {
  await draw(page, `<p>fits</p>`, `<div data-testid="page-nav-bar" style="width:380px;overflow:hidden"><span style="display:block;width:500px">title</span></div>`);
  await expect(expectNoHorizontalOverflow(page)).rejects.toThrow(/reaches 500 > 380/);
});

test("the code editor's own box is measured", async ({ page }) => {
  await draw(page, `<div class="monaco-editor" style="width:350px;height:20px"><div style="width:9000px"></div></div>`);
  await expect(expectNoHorizontalOverflow(page)).rejects.toThrow(/monaco-editor reaches 350 > 300/);
});

test("a box that scrolls sideways fails, but a table's scroll box is noted", async ({ page }) => {
  const scroller = (inner: string) => `<div style="width:200px;overflow-x:auto">${inner}</div>`;
  await draw(page, scroller(`<div style="width:500px;height:20px">tiles</div>`));
  await expect(expectNoHorizontalOverflow(page)).rejects.toThrow(/scrolls sideways/);

  await draw(page, scroller(`<table style="width:500px"><tr><td>cell</td></tr></table>`));
  const noted = await expectNoHorizontalOverflow(page);
  expect(noted).toHaveLength(1);
  expect(noted[0]).toMatch(/scrolls 500 in 200/);
});

test("a page that fits passes, and one with no shown tab returns at once", async ({ page }) => {
  await draw(page, `<div style="width:250px;height:20px">fits</div>`);
  expect(await expectNoHorizontalOverflow(page)).toEqual([]);
  await page.setContent(`<body><p>no tab here</p></body>`);
  const started = Date.now();
  expect(await expectNoHorizontalOverflow(page)).toEqual([]);
  expect(Date.now() - started).toBeLessThan(2_000);
});
