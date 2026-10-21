import { cpSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { installExample, ipc, run, searchable, until } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { newSessionPage, searchLauncher } from "../../support/ui.js";

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
  await expect(newSessionPage(page)).toBeVisible();
  const row = newSessionPage(page).getByRole("row", { name: /Parser work/ });
  await expect(row).toContainText("closed here");
  // A click on the row takes it (its primary action).
  await row.click();

  await expect(page.getByTestId(`center-tab-agent_session:${id}`)).toBeVisible();
  await expect(newSessionPage(page)).toHaveCount(0);
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
  await page.getByTestId("action-lens-action-reopen").click();
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
  const row = newSessionPage(page).getByRole("row", { name: /Lexer work/ });
  await expect(row).toContainText("open in this thread");
  // Its title is a link to the open session's tab.
  await row.getByRole("button", { name: /Lexer work/ }).click();

  await expect(page.getByTestId(`center-tab-body-agent_session:${id}`)).toBeVisible();
  await expect(newSessionPage(page)).toHaveCount(0);
  await expect(page.getByTestId("center-tab-page:new-session")).toHaveCount(1);
});

/// An extension adds a section to the New Session page through its
/// sections slot, placed after "Start a new session" by that section's id,
/// and marked as the extension's.
test("an extension's section lands where its `at` says on the New Session page", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await installExample(daemon, "session-notes");

  // Sections with nothing to show fold into one line, so each has a row:
  // an earlier session and a note.
  const earlier = await run<Opened>(daemon, "oxplow.agent_session.open", {
    thread: `thread:${daemon.thread}`,
    resume: "conv-placed",
    title: "Placed work",
  });
  await run(daemon, "oxplow.agent_session.close", { session: `agent_session:${earlier.result.id}` });
  await run(daemon, "oxplow.knowledge.add_note", { thread: `thread:${daemon.thread}`, body: "Remember the lexer." });

  await page.goto("/");
  const doc = newSessionPage(page);
  await expect(doc.locator("section[data-section]")).toHaveCount(3);
  await expect
    .poll(() => doc.locator("section[data-section]").evaluateAll((s) => s.map((e) => e.getAttribute("data-section"))))
    .toEqual(["start", "session-notes/thread-notes", "earlier"]);
  const notes = doc.getByTestId("lens-part-session-notes/thread-notes");
  await expect(notes.locator("h2")).toContainText("Notes on this thread");
  await expect(notes.locator("[data-source]")).toHaveText("session-notes");
  await expect(notes).toContainText("Remember the lexer.");
});
