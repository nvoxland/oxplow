import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

/// Settings → AI is foundation lenses: a provider is added through a form
/// whose key field is a secret, a role is bound to it from its row, and
/// Remove asks first.
test("a provider is added, a role bound to it, and the provider removed", async ({ page }) => {
  await page.goto("/");
  await openFromLauncher(page, "Settings: AI");
  const providers = page.getByTestId("settings.ai.sections-oxplow-foundation/ai-providers");
  const form = page.getByTestId("settings.ai.sections-oxplow-foundation/ai-provider-form");
  const roles = page.getByTestId("settings.ai.sections-oxplow-foundation/ai-roles");
  await expect(providers).toContainText("No AI providers yet");

  await form.getByTestId("lens-form-fields-id").fill("or");
  await form.getByTestId("lens-form-fields-kind").selectOption("openrouter");
  const key = form.getByTestId("lens-form-fields-key");
  await expect(key).toHaveAttribute("type", "password");
  await key.fill("sk-e2e-secret");
  await form.getByTestId("lens-form-fields-submit").click();

  await expect(providers.getByRole("row", { name: /or\b.*Key saved/ })).toBeVisible();
  await expect(page.locator("body")).not.toContainText("sk-e2e-secret");

  // Bind the summarize role to it from its row.
  await roles.getByRole("row", { name: /summarize/ }).click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-assign").click();
  const assign = page.getByTestId("lens-action-form");
  await assign.getByTestId("lens-action-form-fields-provider").selectOption("or");
  await assign.getByTestId("lens-action-form-fields-model").fill("openai/gpt-5-mini");
  await assign.getByTestId("lens-action-form-fields-submit").click();
  await expect(roles.getByRole("row", { name: /summarize.*or · openai\/gpt-5-mini/ })).toBeVisible();

  // The provider can't go while a role uses it; unassign, then remove (asked first).
  await roles.getByRole("row", { name: /summarize/ }).click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-unassign").click();
  await expect(roles.getByRole("row", { name: /summarize.*Not assigned|summarize.*Falls back/ })).toBeVisible();
  await providers.getByRole("row", { name: /or\b/ }).click({ button: "right" });
  await page.getByTestId("menu-item-lens-action-remove").click();
  await page.getByTestId("lens-action-confirm-run").click();
  await expect(providers).toContainText("No AI providers yet");
});
