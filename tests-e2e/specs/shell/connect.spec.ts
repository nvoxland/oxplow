import { connectedTo, expect, test } from "../../support/fixtures.js";

// tsk971: a wrong token is refused, not a lost daemon — said once, with
// how to fix it, no reconnect loop, no "daemon disconnected" overlay, and
// nothing thrown uncaught.
test("a page holding the wrong token says its token was refused", async ({ browser, baseURL, daemon }) => {
  const context = await browser.newContext({ storageState: connectedTo(baseURL!, daemon.base, "not-the-token") });
  const page = await context.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto("/");
  await expect(page.getByTestId("remote-banner-refused")).toContainText("token was refused");
  await expect(page.getByTestId("remote-banner-down")).toHaveCount(0);
  await expect(page.getByText("Backend daemon disconnected")).toHaveCount(0);
  expect(errors).toEqual([]);
  await context.close();
});

test("a token in the URL fragment wins over the stored one", async ({ browser, baseURL, daemon }) => {
  const context = await browser.newContext({ storageState: connectedTo(baseURL!, daemon.base, "not-the-token") });
  const page = await context.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`/#oxplow-token=${encodeURIComponent(daemon.token)}`);
  await expect(page.getByTestId("page-new-session")).toBeVisible();
  await expect(page.getByTestId("remote-banner-down")).toHaveCount(0);
  expect(errors).toEqual([]);
  await context.close();
});
