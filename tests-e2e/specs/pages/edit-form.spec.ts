import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// A form node in `edit` mode edits what is saved: it starts from the saved
// value, Save is on only while the form is changed, and a markdown field is the
// rich text editor. Over the first stream's prompt (a markdown input).
const spec = `title: Edit Quokka
category: Data
loaders:
  prompt:
    sql: SELECT uri_param(s.uri, 'id') AS stream, coalesce(s.prompt, '') AS prompt FROM v_stream s LIMIT 1
view:
  id: prompt
  viz: form
  from: prompt
  form:
    command: oxplow.stream.set_prompt
    mode: edit
    fields: [prompt]
`;

test("an edit form saves only when changed, takes rich text, and reads back after a reload", async ({ page, daemon }) => {
  await installExtension(daemon, "e2e-edit", {
    manifest: "permissions: [commands:oxplow.stream.set_prompt]\n",
    files: { "pages/quokka.yaml": spec },
  });
  await page.goto("/");
  await openFromLauncher(page, "Edit Quokka");
  const host = page.getByTestId("page-host").filter({ visible: true });
  // The prompt is markdown: the rich text editor's surface.
  const box = host.locator('[contenteditable="true"]');
  const save = host.getByRole("button", { name: "Save" });
  await expect(save).toBeDisabled();
  await box.fill("Always answer in rhyme.");
  await expect(save).toBeEnabled();
  await save.click();
  await expect(save).toBeDisabled();
  await page.reload();
  await expect(host.locator('[contenteditable="true"]')).toHaveText("Always answer in rhyme.");
});
