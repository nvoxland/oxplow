// @desktop-only: the host serves the desktop shell (rail and title bar)
// "Allow other devices" applies at once: the Devices page turns the host's
// listener for other devices on and off while the host runs, with no
// restart.
import { expect, test, type Page } from "@playwright/test";

import { startHost, type Host } from "../../support/host.js";
import { loadScreen, open, shown } from "../../support/ui.js";

let host: Host;
test.beforeAll(async () => {
  // As the Mac app's host: no flags over the saved settings (the port
  // aside, a free one); the address is this machine's loopback, saved
  // beforehand (a test machine may have no private address to offer).
  host = await startHost({ publicBind: false, settings: ["address=127.0.0.1"] });
});
test.afterAll(async () => {
  await host?.stop();
});

/** Pair `page` at the host's home with a code the owner made. */
async function pair(page: Page, name: string) {
  await loadScreen(page, `${host.base}/`, "pair-screen");
  await page.getByTestId("pair-code").fill(await host.pairingCode());
  await page.getByTestId("pair-name").fill(name);
  await page.getByTestId("pair-name").press("Enter");
  await expect(page.getByTestId("host-home")).toBeVisible();
}

test("other devices turn on and off from the Devices page, with no restart", async ({ page }) => {
  test.setTimeout(240_000);
  const id = host.addProject("epsilon");
  await pair(page, "Desk");
  await page.getByTestId(`host-project-open-${id}`).click();
  await expect(page.getByTestId("rail-hud")).toBeVisible({ timeout: 180_000 });
  await open(page, host.project(id), "oxplow:oxplow/page?page=devices");
  const devices = shown(page, "page=devices");
  await expect(devices.getByText("Turn on other devices first.")).toBeVisible();

  const allow = devices.getByLabel("Let other devices use oxplow on this computer");
  const save = devices.getByTestId("viz-form-fields-submit");
  await allow.check();
  await save.click();
  await expect(devices.getByText(/^On: a device opens https:\/\/127\.0\.0\.1:\d+\//)).toBeVisible();
  await expect(devices.getByTestId("viz-action-pair-device")).toBeVisible();
  const status = async () => ((await host.status()) as unknown as { public: { state: string } }).public.state;
  expect(await status()).toBe("listening");

  await allow.uncheck();
  await save.click();
  await expect(devices.getByText("Turn on other devices first.")).toBeVisible();
  await expect(devices.getByText(/^On: a device opens/)).toHaveCount(0);
  expect(await status()).toBe("off");
});
