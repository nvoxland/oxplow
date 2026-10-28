import { run, searchable } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { searchLauncher } from "../../support/ui.js";

test("a wiki page's inline page fence reads live data and refreshes after a command", async ({ page, daemon }) => {
  await run(daemon, "oxplow.knowledge.write_page", {
    slug: "page-block-signal", title: "Signal before refresh", body: "The record the inline view reads.",
  });
  const spec = [
    "title: Live signal",
    "loaders:",
    "  signal:",
    "    sql: SELECT title FROM v_knowledge_page WHERE slug = 'page-block-signal'",
    "view:",
    "  id: signal",
    "  viz: table",
    "  from: signal",
    "  columns: [{ key: title, label: Signal }]",
  ].join("\n");
  await run(daemon, "oxplow.knowledge.write_page", {
    slug: "page-block-live", title: "Page block live",
    body: `# Page block live\n\nBefore the live view.\n\n\`\`\`oxplow-page\n${spec}\n\`\`\`\n\nAfter the live view.\n`,
  });
  await searchable(daemon, "page block live", "wiki");
  await page.goto("/");
  await searchLauncher(page, "page block live");
  await page.getByTestId("launcher-hit-oxplow:oxplow/wiki?slug=page-block-live").click();
  const body = page.locator('[data-node="body"]:visible');
  const inline = body.getByTestId("inline-page");
  await expect(inline).toContainText("Signal before refresh");
  await expect(body.locator(".ProseMirror")).toBeVisible();
  await expect(body).toContainText("Before the live view.");
  await expect(body).toContainText("After the live view.");

  await run(daemon, "oxplow.knowledge.write_page", {
    slug: "page-block-signal", title: "Signal after refresh", body: "The updated record the inline view reads.",
  });
  await expect(inline).toContainText("Signal after refresh");
  await expect(inline).not.toContainText("Signal before refresh");

  // A live block is an island in the rich-text editor. Its pencil opens
  // the stored YAML, and Escape returns to the live view without saving.
  await body.getByRole("button", { name: "Edit YAML" }).click();
  const source = body.locator(".oxplow-rt-page pre code");
  await expect(source).toBeVisible();
  await expect(source).toContainText("SELECT title FROM v_knowledge_page");
  await source.click();
  await page.keyboard.press("Escape");
  await expect(inline).toContainText("Signal after refresh");
  await expect(source).toBeHidden();
});

test("invalid YAML in a wiki page fence shows its source and validation error inline", async ({ page, daemon }) => {
  const invalid = "title: Invalid page\nview: [";
  await run(daemon, "oxplow.knowledge.write_page", {
    slug: "page-block-invalid", title: "Page block invalid",
    body: `# Page block invalid\n\n\`\`\`oxplow-page\n${invalid}\n\`\`\`\n\nThe rest of the page still renders.\n`,
  });
  await searchable(daemon, "page block invalid", "wiki");
  await page.goto("/");
  await searchLauncher(page, "page block invalid");
  await page.getByTestId("launcher-hit-oxplow:oxplow/wiki?slug=page-block-invalid").click();
  const body = page.locator('[data-node="body"]:visible');
  const error = body.getByTestId("inline-page-error");
  await expect(error).toBeVisible();
  await expect(error.locator("pre")).toContainText(invalid);
  await expect(error.locator("p")).toHaveText(/.+/);
  await expect(body).toContainText("The rest of the page still renders.");
});
