import { run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";

test("search finds a task and a wiki page, and opens them", async ({ page, daemon }) => {
  const created = await run<{ result: { id: string } }>(daemon, "work_item.create", {
    title: "Quokka migration",
    native: { thread: daemon.thread },
  });
  await run(daemon, "knowledge.write_page", { slug: "quokka-notes", title: "Quokka notes", body: "# Quokka notes\n\nWhat we know.\n" });
  await searchable(daemon, "quokka", "task");
  await searchable(daemon, "quokka", "wiki");
  await page.goto("/");
  await page.getByTestId("rail-search").click();
  await page.keyboard.type("quokka");
  await page.getByTestId(`launcher-hit-task:${created.result.id}`).click();
  await expect(page.getByTestId("task-rail-delete-trigger")).toBeVisible();
  await expect(page.locator("body")).toContainText("Quokka migration");
  await page.getByTestId("rail-search").click();
  await page.keyboard.type("quokka");
  await page.getByTestId("launcher-hit-wiki:quokka-notes").click();
  await expect(page.getByTestId("page-wiki")).toContainText("What we know.");
});
