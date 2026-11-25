// A browser keeps one window per project: a second tab of the same browser
// takes the project over, the first says so, and Use it here takes it back.
import { expect, test } from "../../support/fixtures.js";
import { load } from "../../support/ui.js";

test("a second tab of the same browser takes over, and Use it here takes it back", async ({ page }) => {
  await load(page);
  await expect(page.getByTestId("replaced-screen")).toBeHidden();

  // The same browser context: the same device.
  const other = await page.context().newPage();
  await load(other);
  await expect(page.getByTestId("replaced-screen")).toBeVisible();
  await expect(page.getByTestId("replaced-screen")).toContainText("open in another tab");
  await expect(other.getByTestId("replaced-screen")).toBeHidden();
  await expect(other.getByTestId("title-bar")).toBeVisible();

  // Use it here loads the first again; it takes the project back.
  await page.getByTestId("replaced-use-here").click();
  await expect(other.getByTestId("replaced-screen")).toBeVisible();
  await expect(page.getByTestId("title-bar")).toBeVisible();
  await expect(page.getByTestId("replaced-screen")).toBeHidden();
  await other.close();
});
