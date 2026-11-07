import { run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { newSessionPage, searchLauncher } from "../../support/ui.js";

interface Opened {
  result: { id: string };
}

/// A thread's closed session is listed on the New Session page as closed
/// here; picking it brings the same session's tab back.
test("a closed session is listed on the New Session page and picking it reopens it", async ({ page, daemon }) => {
  // A session that has run (it is bound to a conversation), then closed.
  const opened = await run<Opened>(daemon, "oxplow.agent_session.open", {
    thread: daemon.thread,
    resume: "conv-earlier",
    title: "Parser work",
  });
  const id = opened.result.id;
  await run(daemon, "oxplow.agent_session.close", { session: id });

  await page.goto("/");
  await expect(newSessionPage(page)).toBeVisible();
  const row = newSessionPage(page).getByRole("row", { name: /Parser work/ });
  await expect(row).toContainText("closed here");
  // A click on the row takes it (its primary action).
  await row.click();

  await expect(page.getByTestId(`center-tab-oxplow:oxplow/agent_session?id=${id}`)).toBeVisible();
  await expect(newSessionPage(page)).toHaveCount(0);
  // Undo (the toast) closes it again.
  await page.getByTestId("undo-toast-undo").click();
  await expect(page.getByTestId(`center-tab-oxplow:oxplow/agent_session?id=${id}`)).not.toBeVisible();
});

/// A closed session found by search opens its transcript, not nothing; the
/// page's Reopen brings the session back.
test("a closed session found by search opens its transcript, which can reopen it", async ({ page, daemon }) => {
  const opened = await run<Opened>(daemon, "oxplow.agent_session.open", {
    thread: daemon.thread,
    resume: "conv-found",
    title: "Zebrafinch review",
  });
  const id = opened.result.id;
  await run(daemon, "oxplow.agent_session.close", { session: id });
  await searchable(daemon, "Zebrafinch", "agent_session");

  await page.goto("/");
  await searchLauncher(page, "Zebrafinch");
  await page.getByTestId(`launcher-hit-oxplow:oxplow/agent_session?id=${id}`).click();
  // Its transcript (it has no turns yet, so nothing is listed), not a tab
  // that isn't there; Reopen brings the session back.
  await expect(page.getByTestId(`center-tab-oxplow:oxplow/agent_session?id=${id}`)).not.toBeVisible();
  const transcript = page.locator('[data-testid^="center-tab-body-oxplow:oxplow/page?page=session-transcript"]');
  await transcript.getByTestId("page-nav-menu").click();
  await page.getByTestId("menu-item-action-page-action-reopen").click();
  await expect(page.getByTestId(`center-tab-oxplow:oxplow/agent_session?id=${id}`)).toBeVisible();
});

/// A session open in this thread is listed too; picking it shows its tab
/// and leaves the New Session page open (only starting one closes it).
test("picking a session open here shows it and keeps the New Session page", async ({ page, daemon }) => {
  const opened = await run<Opened>(daemon, "oxplow.agent_session.open", {
    thread: daemon.thread,
    resume: "conv-open",
    title: "Lexer work",
  });
  const id = opened.result.id;

  await page.goto("/");
  await searchLauncher(page, "new agent session");
  await page.getByTestId("launcher-command-oxplow.agent_session.open").click();
  const row = newSessionPage(page).getByRole("row", { name: /Lexer work/ });
  await expect(row).toContainText("open in this thread");
  // Its title is a link to the open session's tab.
  await row.getByRole("button", { name: /Lexer work/ }).click();

  await expect(page.getByTestId(`center-tab-body-oxplow:oxplow/agent_session?id=${id}`)).toBeVisible();
  await expect(newSessionPage(page)).toHaveCount(0);
  await expect(page.getByTestId("center-tab-oxplow:oxplow/page?page=new-session")).toHaveCount(1);
});
