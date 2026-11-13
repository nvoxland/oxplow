import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { open } from "../../support/ui.js";

const uriOf = (id: string) => `oxplow:oxplow/knowledge?id=${id}&store=oxwiki`;

test("a knowledge page's inline page fence reads live data and refreshes after a command", async ({ page, daemon }) => {
  await run(daemon, "oxwiki.page.write", {
    title: "Page block signal", body: "Signal before refresh",
  });
  const spec = [
    "title: Live signal",
    "loaders:",
    "  signal:",
    "    sql: SELECT excerpt FROM v_oxplow_knowledge_page WHERE id = 'page-block-signal'",
    "view:",
    "  id: signal",
    "  viz: table",
    "  from: signal",
    "  columns: [{ key: excerpt, label: Signal }]",
  ].join("\n");
  await run(daemon, "oxwiki.page.write", {
    title: "Page block live",
    body: `# Page block live\n\nBefore the live view.\n\n\`\`\`oxplow-page\n${spec}\n\`\`\`\n\nAfter the live view.\n`,
  });
  await open(page, daemon, uriOf("page-block-live"));
  const body = page.locator('[data-node="body"]:visible');
  const inline = body.getByTestId("inline-page");
  await expect(inline).toContainText("Signal before refresh");
  await expect(body.locator(".ProseMirror")).toBeVisible();
  await expect(body).toContainText("Before the live view.");
  await expect(body).toContainText("After the live view.");

  await run(daemon, "oxwiki.page.write", {
    id: "page-block-signal", title: "Page block signal", body: "Signal after refresh",
  });
  await expect(inline).toContainText("Signal after refresh");
  await expect(inline).not.toContainText("Signal before refresh");

  // A live block is an island in the rich-text editor. Its pencil opens
  // the stored YAML, and Escape returns to the live view without saving.
  await body.getByRole("button", { name: "Edit YAML" }).click();
  const source = body.locator(".oxplow-rt-page pre code");
  await expect(source).toBeVisible();
  await expect(source).toContainText("SELECT excerpt FROM v_oxplow_knowledge_page");
  await source.click();
  await page.keyboard.press("Escape");
  await expect(inline).toContainText("Signal after refresh");
  await expect(source).toBeHidden();
});

test("invalid YAML in a knowledge page fence shows its source and validation error inline", async ({ page, daemon }) => {
  const invalid = "title: Invalid page\nview: [";
  await run(daemon, "oxwiki.page.write", {
    title: "Page block invalid",
    body: `# Page block invalid\n\n\`\`\`oxplow-page\n${invalid}\n\`\`\`\n\nThe rest of the page still renders.\n`,
  });
  await open(page, daemon, uriOf("page-block-invalid"));
  const body = page.locator('[data-node="body"]:visible');
  const error = body.getByTestId("inline-page-error");
  await expect(error).toBeVisible();
  await expect(error.locator("pre")).toContainText(invalid);
  await expect(error.locator("p")).toHaveText(/.+/);
  await expect(body).toContainText("The rest of the page still renders.");
});
