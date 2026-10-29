import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";

import type { Page } from "@playwright/test";

import { run, type Daemon } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// Oxplow's own pages are complete with every enhancement off: with only
// oxplow-foundation on, a page shows its own content, and nothing another
// extension adds — no replacement standing in for a node, no node an
// extension mounts, no decorator's chip or badge, no panel in the rail, no
// command in the ☰. The pages that read oxplow's own work list (Board, Tasks)
// read the list oxplow-foundation provides: a project that has no work items
// yet shows their empty state, cleanly. The `ui`-less half of
// this — what a page's run holds — is every foundation fixture's, which runs
// with only foundation on.

/** The other extensions that ship with, or are in, the e2e project. */
const OTHERS = ["oxplow-bundled", "e2e", "github"];

/** A few commits (one a merge) and one file not committed. */
function history(daemon: Daemon) {
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.com", ...args], { cwd: daemon.project });
  writeFileSync(join(daemon.project, "main.txt"), "main\n");
  git("add", "main.txt");
  git("commit", "-q", "-m", "Main work");
  git("checkout", "-q", "-b", "side");
  writeFileSync(join(daemon.project, "side.txt"), "side\n");
  git("add", "side.txt");
  git("commit", "-q", "-m", "Side work");
  git("checkout", "-q", "-");
  git("merge", "-q", "--no-ff", "-m", "Merge side", "side");
  writeFileSync(join(daemon.project, "dirty.txt"), "dirty\n");
}

/** Nothing of another extension's is on the page the window shows or in its ☰. */
async function nothingAdded(page: Page) {
  await expect(page.locator('[data-testid^="replacement-"]')).toHaveCount(0);
  for (const name of OTHERS) await expect(page.locator(`[data-node^="${name}."]`)).toHaveCount(0);
  await expect(page.locator('[title^="from "]')).toHaveCount(0);
  await expect(
    page.locator('[data-testid^="rail-section-ext:"]:not([data-testid^="rail-section-ext:oxplow-foundation/"])'),
  ).toHaveCount(0);
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await expect(page.getByTestId("menu-item-page-ask-assistant")).toBeVisible();
  await expect(page.locator('[data-testid^="menu-item-ref-command-"]')).toHaveCount(0);
  await page.keyboard.press("Escape");
}

test("oxplow's pages are complete with every enhancement off", async ({ fresh }) => {
  const { page, daemon } = fresh;
  history(daemon);
  for (const name of OTHERS) await run(daemon, "oxplow.extension.disable", { name });
  await page.goto("/");
  const host = page.locator('[data-testid="page-host"]:visible');

  // The VCS pages: their own content.
  await openFromLauncher(page, "Git History");
  await expect(host.getByText("Merge side").first()).toBeVisible();
  await expect(host.getByText("Side work").first()).toBeVisible();
  await nothingAdded(page);

  await host.getByText("Main work").first().click();
  const commit = page.locator('[data-testid="page-file"]:visible');
  await expect(commit).toContainText("Main work");
  await expect(commit).toContainText("main.txt");
  await nothingAdded(page);

  await openFromLauncher(page, "Uncommitted");
  await expect(host.getByText("dirty.txt").first()).toBeVisible();
  await nothingAdded(page);

  // The work-list pages: the list is empty, so they say so, with no error.
  await openFromLauncher(page, "Board");
  await expect(host.getByText("No work items here.")).toBeVisible();
  await expect(host.getByRole("alert")).toHaveCount(0);
  await nothingAdded(page);

  await openFromLauncher(page, "Tasks");
  await expect(page.getByTestId("page-nav-title").filter({ visible: true })).toHaveText("Tasks");
  await expect(host.getByRole("alert")).toHaveCount(0);
  await nothingAdded(page);
});
