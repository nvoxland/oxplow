import { ipc, run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { installExtension } from "../../support/pages.js";
import { newSessionPage, searchLauncher } from "../../support/ui.js";

test("an agent terminal has usable height when opened and restored", async ({ page }) => {
  await page.goto("/");
  await newSessionPage(page).locator('[data-node="start"]').getByRole("button", { name: "Start a new session", exact: true }).click();
  const terminal = page.getByTestId("viz-terminal");
  const hasSpace = async () => {
    await expect(terminal).toBeVisible();
    await expect.poll(async () => (await terminal.boundingBox())?.height ?? 0).toBeGreaterThan(200);
  };
  await hasSpace();
  await page.reload();
  await hasSpace();
});

/// Search offers New Agent Session…; choosing it opens the session picker
/// rather than starting anything. On a fresh daemon: the picker is the home
/// tab only while the thread has no session, which another spec on the
/// shared daemon may have started.
test("search offers a new agent session, which opens the picker", async ({ page }) => {
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await newSessionPage(page).locator('[data-node="start"]').getByRole("button", { name: "Start a new session", exact: true }).click();
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
test("an ended terminal session says so and its tab closes it", async ({ page, daemon }) => {
  await run(daemon, "oxplow.agent.set_agents", { agents: ["scripted", "claude"] });
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await newSessionPage(page).locator('[data-node="start"]').getByTestId("viz-form-fields-harness").selectOption("claude");
  await newSessionPage(page).locator('[data-node="start"]').getByRole("button", { name: "Start a new session", exact: true }).click();
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
test("a session that can't launch shows why and offers a retry", async ({ page, daemon }) => {
  await run(daemon, "oxplow.agent.set_agents", { agents: ["scripted", "codex"] });
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await newSessionPage(page).locator('[data-node="start"]').getByTestId("viz-form-fields-harness").selectOption("codex");
  await newSessionPage(page).locator('[data-node="start"]').getByRole("button", { name: "Start a new session", exact: true }).click();
  const ended = page.getByTestId("terminal-ended");
  await expect(ended).toContainText("Couldn't start the session");
  await expect(ended).toContainText("codex");
  await expect(page.getByTestId("terminal-retry")).toBeVisible();
});

/// Extensions reloading (a deploy, here) says nothing against an open tab: its
/// page and terminal stay mounted, so a menu open on it stays open.
test("an open tab stays drawn while the extensions reload", async ({ page, daemon }) => {
  await run(daemon, "oxplow.agent.set_agents", { agents: ["scripted", "claude", "codex"] });
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await newSessionPage(page).locator('[data-node="start"]').getByTestId("viz-form-fields-harness").selectOption("claude");
  await newSessionPage(page).locator('[data-node="start"]').getByRole("button", { name: "Start a new session", exact: true }).click();
  const tab = page.locator('[data-testid^="center-tab-oxplow:oxplow/agent_session?"]').first();
  await expect(tab).toBeVisible();
  const id = ((await tab.getAttribute("data-testid")) ?? "").slice("center-tab-".length);
  const body = page.getByTestId(`center-tab-body-${id}`);
  await body.getByTestId("page-nav-menu").click();
  const rename = page.getByTestId("menu-item-action-page-action-rename");
  await expect(rename).toBeVisible();

  const askedAgain = page.waitForResponse((r) => r.url().endsWith("/ipc/renderers_for"));
  await installExtension(daemon, "e2e-reload", {
    manifest: "",
    files: { "pages/e2e-reload.yaml": "title: Reload\nloaders:\n  n:\n    sql: SELECT 1 AS n\nview: { id: n, viz: number, from: n }\n" },
  });
  await askedAgain;
  await expect(rename).toBeVisible();
});

/// A session's tab is named by the view's one rule: the person's alias, else
/// the harness's name for the conversation, else the harness and the day.
/// Clearing the alias gives the fallback back.
test("a session's tab shows its alias, and its fallback once the alias is cleared", async ({ page, daemon }) => {
  await run(daemon, "oxplow.agent.set_agents", { agents: ["scripted", "claude", "codex"] });
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await newSessionPage(page).locator('[data-node="start"]').getByTestId("viz-form-fields-harness").selectOption("claude");
  await newSessionPage(page).locator('[data-node="start"]').getByRole("button", { name: "Start a new session", exact: true }).click();
  const tab = page.locator('[data-testid^="center-tab-oxplow:oxplow/agent_session?"]').first();
  await expect(tab).toBeVisible();
  await expect(tab).toContainText(/claude \d{4}-\d{2}-\d{2}/i);
  const id = ((await tab.getAttribute("data-testid")) ?? "").slice("center-tab-".length);
  const session = new URL(id).searchParams.get("id");
  const rename = (title: string) => run(daemon, "oxplow.agent_session.rename", { session, title });
  await rename("My review");
  await expect(tab).toContainText("My review");
  // Its page (oxplow-foundation's `agent-session` document) names it too,
  // offers its actions, and has no back or forward: the tab is pinned.
  const body = page.getByTestId(`center-tab-body-${id}`);
  await expect(body.getByTestId("page-nav-title")).toHaveText("My review");
  await expect(body.getByTestId("page-nav-back")).toHaveCount(0);
  // Rename… is in its ☰, and is a form: the title, and the box to rename
  // it in its harness.
  await body.getByTestId("page-nav-menu").click();
  // Reopen Session is about a session (`ui.about`) but only a closed one
  // (`about_when`): this one is open, so it isn't offered, and the ☰ has no
  // "Tasks" submenu for the commands about the session.
  await expect(page.getByTestId("menu-item-action-page-action-rename")).toBeVisible();
  await expect(page.getByTestId("menu-item-ref-command-oxplow.agent_session.reopen")).toHaveCount(0);
  await expect(page.getByTestId("menu-item-page-submenu-Tasks")).toHaveCount(0);
  await body.getByTestId("menu-item-action-page-action-rename").click();
  // The form is the tab's modal: headed without the menu's ellipsis, about
  // the session by its name (never its id), its Name starting on the
  // current one, with a primary submit and a Cancel.
  const form = body.getByTestId("viz-action-form");
  await expect(form.getByTestId("viz-action-form-title")).toHaveText("Rename");
  await expect(form.getByTestId("viz-action-form-subtitle")).toHaveText("My review");
  await expect(form).not.toContainText(/ses\d/);
  const title = page.getByTestId("viz-action-form-fields-title");
  await expect(title).toHaveValue("My review");
  await expect(form.getByTestId("viz-action-form-fields-submit")).toHaveText("Rename");
  await expect(form.getByTestId("viz-action-form-fields-cancel")).toBeVisible();
  await title.fill("Renamed here");
  // Another tab is fully usable with this one's modal open, and the modal
  // (with what was typed) is still there when the person comes back.
  await searchLauncher(page, "new codex session");
  await page.getByTestId("launcher-command-oxplow.agent_session.open_codex").click();
  const sessions = page.locator('[data-testid^="center-tab-oxplow:oxplow/agent_session?"]');
  await expect(sessions).toHaveCount(2);
  await sessions.nth(1).click();
  await expect(body).toBeHidden();
  await tab.click();
  await expect(page.getByTestId("viz-action-form")).toBeVisible();
  await expect(page.getByTestId("viz-action-form-fields-title")).toHaveValue("Renamed here");
  await page.getByTestId("viz-action-form-fields-submit").click();
  await expect(tab).toContainText("Renamed here");
  await rename("");
  await expect(tab).toContainText(/claude \d{4}-\d{2}-\d{2}/i);
});

/// Search offers a command per enabled agent that starts it directly — the
/// project's agents decide which: Codex once enabled, never Claude here.
test("search starts an enabled agent's session directly", async ({ page, daemon }) => {
  await run(daemon, "oxplow.agent.set_agents", { agents: ["scripted", "codex"] });
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
test("the picker closes, and stays closed", async ({ page, daemon }) => {
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await page.getByTestId(PICKER_CLOSE).click();
  await expect(newSessionPage(page)).toHaveCount(0);
  await page.reload();
  await expect(page.getByTestId("navigator-strip")).toBeVisible();
  await expect(page.getByTestId(PICKER_CLOSE)).toHaveCount(0);
});

/// Remember This makes the agent chosen what new threads start with.
test("a remembered choice is what new threads start with", async ({ page, daemon }) => {
  await run(daemon, "oxplow.agent.set_agents", { agents: ["scripted", "claude"] });
  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  await newSessionPage(page).locator('[data-node="start"]').getByTestId("viz-form-fields-harness").selectOption("claude");
  await newSessionPage(page).locator('[data-node="start"]').getByTestId("viz-form-fields-remember").check();
  await newSessionPage(page).locator('[data-node="start"]').getByRole("button", { name: "Start a new session", exact: true }).click();
  await expect(page.locator('[data-testid^="center-tab-oxplow:oxplow/agent_session?"]')).toHaveCount(1);

  await addThread(page, daemon, "Busy");
  await expect(page.locator('[data-testid^="center-tab-oxplow:oxplow/agent_session?"]')).toHaveCount(1);
  await expect(page.getByTestId(PICKER_CLOSE)).toHaveCount(0);
  await expect(page.getByTestId("terminal-ended")).toBeVisible();
});
