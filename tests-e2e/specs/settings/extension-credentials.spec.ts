import { installExample } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// Settings → Extensions lists the credentials extensions' collectors
// declare, from the foundation's `extensions-credentials` lens. Set… asks
// for the value in a masked field; it goes to the keychain, and the row
// says only that one is saved.
test("a collector's credential is set from its row and cleared again", async ({ page, daemon }) => {
  await installExample(daemon, "github");
  await page.goto("/");
  await openFromLauncher(page, "Settings: Extensions");
  const section = page.locator('[data-node="extensions-credentials"]').filter({ visible: true });
  const row = section.getByRole("row", { name: /GITHUB_TOKEN/ });
  await expect(row).toContainText("Not set");

  await row.click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-set").click();
  const form = page.getByTestId("lens-action-form");
  const value = form.getByTestId("lens-action-form-fields-value");
  await expect(value).toHaveAttribute("type", "password");
  await value.fill("ghp_example_value");
  await form.getByTestId("lens-action-form-fields-submit").click();
  await expect(row).toContainText("Saved in keychain");
  await expect(page.locator("body")).not.toContainText("ghp_example_value");

  await row.click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-clear").click();
  await expect(row).toContainText("Not set");
});
