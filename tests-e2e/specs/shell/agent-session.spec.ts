import { ipc } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";

/// A session opened from the picker gets its pinned tab; its × arms a
/// "Close session" confirm, and confirming closes the session — its tab
/// goes and the thread, with no session left, shows the picker again.
test("a session's tab closes the session once its close is confirmed", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("page-new-session")).toBeVisible();
  await page.getByTestId("new-session-start").click();
  const tab = page.locator('[data-testid^="center-tab-agent_session:"]').first();
  await expect(tab).toBeVisible();
  const tabTestId = (await tab.getAttribute("data-testid")) ?? "";
  const id = tabTestId.slice("center-tab-".length);
  await page.getByTestId(`center-tab-close-${id}`).click();
  await page.getByTestId(`center-tab-close-${id}-confirm`).click();
  await expect(page.getByTestId(tabTestId)).toHaveCount(0);
  await expect(page.getByTestId("page-new-session")).toBeVisible();
});

/// Search offers New Agent Session…; choosing it opens the session picker
/// rather than starting anything.
test("search offers a new agent session, which opens the picker", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("page-new-session")).toBeVisible();
  await page.getByTestId("new-session-start").click();
  const tab = page.locator('[data-testid^="center-tab-agent_session:"]').first();
  await expect(tab).toBeVisible();
  await expect(page.getByTestId("page-new-session")).toBeHidden();
  await page.getByTestId("title-bar-search").click();
  await page.keyboard.type("new agent session");
  await page.getByTestId("launcher-command-oxplow.agent_session.open").click();
  await expect(page.getByTestId("page-new-session")).toBeVisible();
  // The session it started is still there: choosing it started nothing.
  await expect(tab).toBeVisible();
});

/// A terminal agent whose process has ended (here Codex, which isn't
/// installed where the daemon looks, so it exits at once) shows "Session
/// ended", and its tab still closes the session.
test("an ended terminal session says so and its tab closes it", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await ipc(daemon, "set_agents", { agents: ["acp", "codex"] });
  await page.goto("/");
  await expect(page.getByTestId("page-new-session")).toBeVisible();
  await page.getByTestId("new-session-agent").selectOption("codex");
  await page.getByTestId("new-session-start").click();
  const tab = page.locator('[data-testid^="center-tab-agent_session:"]').first();
  await expect(tab).toBeVisible();
  await expect(page.getByTestId("terminal-ended")).toBeVisible();
  const tabTestId = (await tab.getAttribute("data-testid")) ?? "";
  const id = tabTestId.slice("center-tab-".length);
  await page.getByTestId(`center-tab-close-${id}`).click();
  await page.getByTestId(`center-tab-close-${id}-confirm`).click();
  await expect(page.getByTestId(tabTestId)).toHaveCount(0);
});
