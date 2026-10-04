import { connectedTo, expect, test } from "../../support/fixtures.js";

test("a page holding the wrong token says it lost the daemon", async ({ browser, baseURL, daemon }) => {
  const context = await browser.newContext({ storageState: connectedTo(baseURL!, daemon.base, "not-the-token") });
  const page = await context.newPage();
  await page.goto("/");
  await expect(page.getByTestId("remote-banner-down")).toBeVisible();
  await context.close();
});

test("a token in the URL fragment wins over the stored one", async ({ browser, baseURL, daemon }) => {
  const context = await browser.newContext({ storageState: connectedTo(baseURL!, daemon.base, "not-the-token") });
  const page = await context.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`/#oxplow-token=${encodeURIComponent(daemon.token)}`);
  await expect(page.getByTestId("page-agent")).toBeVisible();
  await expect(page.getByTestId("remote-banner-down")).toHaveCount(0);
  expect(errors).toEqual([]);
  await context.close();
});
