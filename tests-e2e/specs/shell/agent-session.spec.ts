import { ipc, run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { newSessionPage, searchLauncher } from "../../support/ui.js";

/// Search offers New Agent Session…; choosing it opens the session picker
/// rather than starting anything. On a fresh daemon: the picker is the home
/// tab only while the thread has no session, which another spec on the
/// shared daemon may have started.
test("search offers a new agent session, which opens the picker", async ({ fresh }) => {
  const { page } = fresh;
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await newSessionPage(page).getByTestId("lens-part-start").getByRole("button", { name: "New Session", exact: true }).click();
  const tab = page.locator('[data-testid^="center-tab-oxplow:oxplow/agent_session?"]').first();
  await expect(tab).toBeVisible();
  await expect(newSessionPage(page)).toHaveCount(0);
  await searchLauncher(page, "new agent session");
  await page.getByTestId("launcher-command-oxplow.agent_session.open").click();
  await expect(newSessionPage(page)).toBeVisible();
  // The session it started is still there: choosing it started nothing.
  await expect(tab).toBeVisible();
});

/// A terminal agent whose process has ended (here Claude Code, which isn't
/// installed where the daemon looks — its PATH leaves out the person's own
/// bin dirs — so its launch shell exits at once)
/// shows "Session ended", and its tab still closes the session.
test("an ended terminal session says so and its tab closes it", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await run(daemon, "oxplow.agent.set_agents", { agents: ["terminal", "claude"] });
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await newSessionPage(page).getByTestId("lens-part-start").getByTestId("lens-form-fields-harness").selectOption("claude");
  await newSessionPage(page).getByTestId("lens-part-start").getByRole("button", { name: "New Session", exact: true }).click();
  const tab = page.locator('[data-testid^="center-tab-oxplow:oxplow/agent_session?"]').first();
  await expect(tab).toBeVisible();
  await expect(page.getByTestId("terminal-ended")).toBeVisible();
  const tabTestId = (await tab.getAttribute("data-testid")) ?? "";
  const id = tabTestId.slice("center-tab-".length);
  await page.getByTestId(`center-tab-close-${id}`).click();
  await expect(page.getByTestId(tabTestId)).toHaveCount(0);
});

/// A launch that fails before any process exists (Codex, which the daemon's
/// PATH can't find) says why where "Session ended" shows, with a retry — not
/// a blank pane.
test("a session that can't launch shows why and offers a retry", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await run(daemon, "oxplow.agent.set_agents", { agents: ["terminal", "codex"] });
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await newSessionPage(page).getByTestId("lens-part-start").getByTestId("lens-form-fields-harness").selectOption("codex");
  await newSessionPage(page).getByTestId("lens-part-start").getByRole("button", { name: "New Session", exact: true }).click();
  const ended = page.getByTestId("terminal-ended");
  await expect(ended).toContainText("Couldn't start the session");
  await expect(ended).toContainText("codex");
  await expect(page.getByTestId("terminal-retry")).toBeVisible();
});

/// A session's tab is named by the view's one rule: the person's alias, else
/// the harness's name for the conversation, else the harness and the day.
/// Clearing the alias gives the fallback back.
test("a session's tab shows its alias, and its fallback once the alias is cleared", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await run(daemon, "oxplow.agent.set_agents", { agents: ["terminal", "claude"] });
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await newSessionPage(page).getByTestId("lens-part-start").getByTestId("lens-form-fields-harness").selectOption("claude");
  await newSessionPage(page).getByTestId("lens-part-start").getByRole("button", { name: "New Session", exact: true }).click();
  const tab = page.locator('[data-testid^="center-tab-oxplow:oxplow/agent_session?"]').first();
  await expect(tab).toBeVisible();
  await expect(tab).toContainText(/claude \d{4}-\d{2}-\d{2}/i);
  const id = ((await tab.getAttribute("data-testid")) ?? "").slice("center-tab-".length);
  const session = new URL(id).searchParams.get("id");
  const rename = (title: string) => run(daemon, "oxplow.agent_session.rename", { session, title });
  await rename("My review");
  await expect(tab).toContainText("My review");
  // Its page's bar names it too, beside its harness's logo, offers its
  // actions, and has no back or forward: the tab is pinned.
  const body = page.getByTestId(`center-tab-body-${id}`);
  await expect(body.getByTestId("page-nav-title")).toHaveText("My review");
  await expect(body.getByTestId("page-nav-title-icon")).toBeVisible();
  await expect(body.getByTestId("page-nav-back")).toHaveCount(0);
  await expect(body.getByTestId("action-agent-session.rename")).toBeVisible();
  await rename("");
  await expect(tab).toContainText(/claude \d{4}-\d{2}-\d{2}/i);
});

/// Search offers a command per enabled agent that starts it directly — the
/// project's agents decide which: Codex once enabled, never Claude here.
test("search starts an enabled agent's session directly", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await run(daemon, "oxplow.agent.set_agents", { agents: ["terminal", "codex"] });
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await searchLauncher(page, "new claude session");
  await expect(page.getByTestId("launcher-command-oxplow.agent_session.open_claude")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await searchLauncher(page, "new codex session");
  await page.getByTestId("launcher-command-oxplow.agent_session.open_codex").click();
  const tab = page.locator('[data-testid^="center-tab-oxplow:oxplow/agent_session?"]').first();
  await expect(tab).toBeVisible();
  await expect(newSessionPage(page)).toHaveCount(0);
});

const PICKER_CLOSE = "center-tab-close-oxplow:oxplow/page?page=new-session";

/** A new thread from the navigator: the stream's menu, Add thread, a title. */
async function addThread(page: import("@playwright/test").Page, daemon: import("../../support/daemon.js").Daemon, title: string) {
  const [stream] = await ipc<{ id: string }[]>(daemon, "list_streams");
  await page.getByTestId(`navigator-strip-stream-${stream!.id}`).click({ button: "right" });
  await page.getByTestId("menu-item-stream.add-thread").click();
  await page.getByTestId("navigator-new-thread-input").fill(title);
  await page.keyboard.press("Enter");
}

/// The picker is an ordinary tab: its × closes it for good (a reload
/// doesn't bring it back).
test("the picker closes, and stays closed", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await page.getByTestId(PICKER_CLOSE).click();
  await expect(newSessionPage(page)).toHaveCount(0);
  await page.reload();
  await expect(page.getByTestId("navigator-strip")).toBeVisible();
  await expect(page.getByTestId(PICKER_CLOSE)).toHaveCount(0);
});

/// Remember This makes the agent chosen what new threads start with.
test("a remembered choice is what new threads start with", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await run(daemon, "oxplow.agent.set_agents", { agents: ["terminal", "claude"] });
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await newSessionPage(page).getByTestId("lens-part-start").getByTestId("lens-form-fields-harness").selectOption("claude");
  await newSessionPage(page).getByTestId("lens-part-start").getByTestId("lens-form-fields-remember").check();
  await newSessionPage(page).getByTestId("lens-part-start").getByRole("button", { name: "New Session", exact: true }).click();
  await expect(page.locator('[data-testid^="center-tab-oxplow:oxplow/agent_session?"]')).toHaveCount(1);

  await addThread(page, daemon, "Busy");
  await expect(page.locator('[data-testid^="center-tab-oxplow:oxplow/agent_session?"]')).toHaveCount(1);
  await expect(page.getByTestId(PICKER_CLOSE)).toHaveCount(0);
  await expect(page.getByTestId("terminal-ended")).toBeVisible();
});
