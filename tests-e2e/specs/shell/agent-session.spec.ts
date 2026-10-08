import { ipc } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";

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

/// Search offers a command per enabled agent that starts it directly — the
/// project's agents decide which: Codex once enabled, never Claude here.
test("search starts an enabled agent's session directly", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await ipc(daemon, "set_agents", { agents: ["acp", "codex"] });
  await page.goto("/");
  await expect(page.getByTestId("page-new-session")).toBeVisible();
  await page.getByTestId("title-bar-search").click();
  await page.keyboard.type("new claude session");
  await expect(page.getByTestId("launcher-command-oxplow.agent_session.open_claude")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.getByTestId("title-bar-search").click();
  await page.keyboard.type("new codex session");
  await page.getByTestId("launcher-command-oxplow.agent_session.open_codex").click();
  const tab = page.locator('[data-testid^="center-tab-agent_session:"]').first();
  await expect(tab).toBeVisible();
  await expect(page.getByTestId("page-new-session")).toBeHidden();
});

const PICKER_CLOSE = "center-tab-close-page:new-session";

/** A new thread from the navigator: the stream's menu, Add thread, a title. */
async function addThread(page: import("@playwright/test").Page, daemon: import("../../support/daemon.js").Daemon, title: string) {
  const [stream] = await ipc<{ id: string }[]>(daemon, "list_streams");
  await page.getByTestId(`navigator-strip-stream-${stream!.id}`).click({ button: "right" });
  await page.getByTestId("menu-item-stream.add-thread").click();
  await page.getByTestId("navigator-new-thread-input").fill(title);
  await page.keyboard.press("Enter");
}

/// The picker is an ordinary tab: its × closes it for good (a reload
/// doesn't bring it back), and No Session in This Thread does the same.
test("the picker closes, and stays closed", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await page.goto("/");
  await expect(page.getByTestId("page-new-session")).toBeVisible();
  await page.getByTestId(PICKER_CLOSE).click();
  await expect(page.getByTestId("page-new-session")).toBeHidden();
  await page.reload();
  await expect(page.getByTestId("navigator-strip")).toBeVisible();
  await expect(page.getByTestId(PICKER_CLOSE)).toHaveCount(0);

  await addThread(page, daemon, "Research");
  await expect(page.getByTestId("page-new-session")).toBeVisible();
  await page.getByTestId("new-session-none").click();
  await expect(page.getByTestId(PICKER_CLOSE)).toHaveCount(0);
});

/// Remember This makes the choice what new threads start with: none opens
/// no picker; an agent starts that agent's session.
test("a remembered choice is what new threads start with", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await ipc(daemon, "set_agents", { agents: ["acp", "codex"] });
  await page.goto("/");
  await expect(page.getByTestId("page-new-session")).toBeVisible();
  await page.getByTestId("new-session-remember").check();
  await page.getByTestId("new-session-none").click();
  await expect(page.getByTestId(PICKER_CLOSE)).toHaveCount(0);

  await addThread(page, daemon, "Quiet");
  await expect(page.getByTestId("navigator-strip").getByTitle("Quiet")).toBeVisible();
  await expect(page.getByTestId(PICKER_CLOSE)).toHaveCount(0);
  await expect(page.locator('[data-testid^="center-tab-agent_session:"]')).toHaveCount(0);

  // Remember an agent from the picker, which the launcher still opens.
  await page.getByTestId("title-bar-search").click();
  await page.keyboard.type("new agent session");
  await page.getByTestId("launcher-command-oxplow.agent_session.open").click();
  await page.getByTestId("new-session-agent").selectOption("codex");
  await page.getByTestId("new-session-remember").check();
  await page.getByTestId("new-session-start").click();
  await expect(page.locator('[data-testid^="center-tab-agent_session:"]')).toHaveCount(1);

  await addThread(page, daemon, "Busy");
  await expect(page.locator('[data-testid^="center-tab-agent_session:"]')).toHaveCount(1);
  await expect(page.getByTestId(PICKER_CLOSE)).toHaveCount(0);
  await expect(page.getByTestId("terminal-ended")).toBeVisible();
});
