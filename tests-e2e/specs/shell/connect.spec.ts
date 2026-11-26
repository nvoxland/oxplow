import { connectContext, expect, test } from "../../support/fixtures.js";

// a wrong token is refused, not a lost daemon — said once, with
// how to fix it, no reconnect loop, no "daemon disconnected" overlay, and
// nothing thrown uncaught.
test("a page holding the wrong token says its token was refused", async ({ browser, baseURL, daemon }) => {
  const context = await browser.newContext({ baseURL });
  await connectContext(context, daemon.base, "not-the-token");
  const page = await context.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto("/");
  await expect(page.getByTestId("remote-banner-refused")).toContainText("token was refused");
  await expect(page.getByTestId("remote-banner-down")).not.toBeVisible();
  await expect(page.getByText("Backend daemon disconnected")).not.toBeVisible();
  expect(errors).toEqual([]);
  await context.close();
});
