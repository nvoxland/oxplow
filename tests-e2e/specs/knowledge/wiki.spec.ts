import { run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";

test("a written page renders, and a task link in it opens the task", async ({ page, daemon }) => {
  const created = await run<{ result: { id: string } }>(daemon, "oxplow.work_item.create", {
    title: "Wombat follow-up",
    thread: daemon.thread,
  });
  await run(daemon, "oxplow.knowledge.write_page", {
    slug: "wombat-plan",
    title: "Wombat plan",
    body: `# Wombat plan\n\nFirst do [[${created.result.id}]].\n`,
  });
  await searchable(daemon, "wombat", "wiki");
  await page.goto("/");
  await page.getByTestId("title-bar-search").click();
  await page.keyboard.type("wombat plan");
  await page.getByTestId("launcher-hit-wiki:wombat-plan").click();
  const wiki = page.getByTestId("page-wiki");
  await expect(wiki).toContainText("First do");
  await wiki.locator(`a[href="work_item:oxplow:${created.result.id}"]`).click();
  await expect(page.getByTestId("task-rail-delete-trigger")).toBeVisible();
  await expect(page.locator("body")).toContainText("Wombat follow-up");
});
