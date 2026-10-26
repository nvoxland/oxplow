import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

/// Settings → AI is the foundation Settings page's AI section: a provider is added through a form
/// whose key field is a secret, a role is bound to it from its row, and
/// Remove asks first.
test("a provider is added, a role bound to it, and the provider removed", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Settings: AI");
  // With no provider the list has nothing to show: it folds into the "Nothing found" line.
  const providers = page.locator('[data-node="ai-providers"]');
  const form = page.locator('[data-node="ai-provider-form"]');
  const roles = page.locator('[data-node="ai-roles"]');
  await expect(page.getByTestId("page-nothing-found")).toContainText("AI providers");

  await form.getByTestId("viz-form-fields-id").fill("or");
  await form.getByTestId("viz-form-fields-kind").selectOption("openrouter");
  const key = form.getByTestId("viz-form-fields-key");
  await expect(key).toHaveAttribute("type", "password");
  await key.fill("sk-e2e-secret");
  await form.getByTestId("viz-form-fields-submit").click();

  await expect(providers.getByRole("row", { name: /or\b.*Key saved/ })).toBeVisible();
  await expect(page.locator("body")).not.toContainText("sk-e2e-secret");

  // Bind the summarize role to it from its row.
  await roles.getByRole("row", { name: /summarize/ }).click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-assign").click();
  const assign = page.getByTestId("viz-action-form");
  await assign.getByTestId("viz-action-form-fields-provider").selectOption("or");
  await assign.getByTestId("viz-action-form-fields-model").fill("openai/gpt-5-mini");
  await assign.getByTestId("viz-action-form-fields-submit").click();
  await expect(roles.getByRole("row", { name: /summarize.*or · openai\/gpt-5-mini/ })).toBeVisible();

  // The provider can't go while a role uses it; unassign, then remove (asked first).
  await roles.getByRole("row", { name: /summarize/ }).click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-unassign").click();
  await expect(roles.getByRole("row", { name: /summarize.*Not assigned|summarize.*Falls back/ })).toBeVisible();
  await providers.getByRole("row", { name: /or\b/ }).click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-remove").click();
  await page.getByTestId("viz-action-confirm-run").click();
  await expect(page.getByTestId("page-nothing-found")).toContainText("AI providers");
});
