import { run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { searchLauncher } from "../../support/ui.js";

interface Opened {
  result: { id: string };
}

/// A thread's closed session is listed on the New Session page as closed
/// here; picking it brings the same session's tab back.
test("a closed session is listed on the New Session page and picking it reopens it", async ({ fresh }) => {
  const { page, daemon } = fresh;
  // A session that has run (it is bound to a conversation), then closed.
  const opened = await run<Opened>(daemon, "oxplow.agent_session.open", {
    thread: `thread:${daemon.thread}`,
    resume: "conv-earlier",
    title: "Parser work",
  });
  const id = opened.result.id;
  await run(daemon, "oxplow.agent_session.close", { session: `agent_session:${id}` });

  await page.goto("/");
  await expect(page.getByTestId("page-new-session")).toBeVisible();
  const row = page.getByTestId(`earlier-row-agent_session:${id}`);
  await expect(row).toContainText("Parser work");
  await expect(row.locator("[data-status]")).toHaveAttribute("title", "closed here");
  await row.click();

  await expect(page.getByTestId(`center-tab-agent_session:${id}`)).toBeVisible();
  await expect(page.getByTestId("page-new-session")).toHaveCount(0);
  // Undo (the toast) closes it again.
  await page.getByTestId("undo-toast-undo").click();
  await expect(page.getByTestId(`center-tab-agent_session:${id}`)).toHaveCount(0);
});

/// A closed session found by search opens its transcript, not nothing; the
/// lens's Reopen brings the session back.
test("a closed session found by search opens its transcript, which can reopen it", async ({ fresh }) => {
  const { page, daemon } = fresh;
  const opened = await run<Opened>(daemon, "oxplow.agent_session.open", {
    thread: `thread:${daemon.thread}`,
    resume: "conv-found",
    title: "Zebrafinch review",
  });
  const id = opened.result.id;
  await run(daemon, "oxplow.agent_session.close", { session: `agent_session:${id}` });
  await searchable(daemon, "Zebrafinch", "agent_session");

  await page.goto("/");
  await searchLauncher(page, "Zebrafinch");
  await page.getByTestId(`launcher-hit-agent_session:${id}`).click();
  // Its transcript (it has no turns yet, so nothing is listed), not a tab
  // that isn't there; Reopen brings the session back.
  await expect(page.getByTestId(`center-tab-agent_session:${id}`)).toHaveCount(0);
  await page.getByTestId("lens-action-reopen").click();
  await expect(page.getByTestId(`center-tab-agent_session:${id}`)).toBeVisible();
});

/// A session open in this thread is listed too; picking it shows its tab
/// and leaves the New Session page open (only starting one closes it).
test("picking a session open here shows it and keeps the New Session page", async ({ fresh }) => {
  const { page, daemon } = fresh;
  const opened = await run<Opened>(daemon, "oxplow.agent_session.open", {
    thread: `thread:${daemon.thread}`,
    resume: "conv-open",
    title: "Lexer work",
  });
  const id = opened.result.id;

  await page.goto("/");
  await searchLauncher(page, "new agent session");
  await page.getByTestId("launcher-command-oxplow.agent_session.open").click();
  const row = page.getByTestId(`earlier-row-agent_session:${id}`);
  await expect(row.locator("[data-status]")).toHaveAttribute("title", "open in this thread");
  await row.click();

  await expect(page.getByTestId(`center-tab-body-agent_session:${id}`)).toBeVisible();
  await expect(page.getByTestId("page-new-session")).toBeHidden();
  await expect(page.getByTestId("center-tab-page:new-session")).toHaveCount(1);
});
