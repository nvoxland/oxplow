import type { Page } from "@playwright/test";

import { run } from "../../support/daemon.js";
import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// What any page's actions do in the shell, over one small extension: a row's
// right-click menu opens a form (a modal, or a prompt strip when it asks for
// one field) and submits it; a command that asks is confirmed in a strip; a
// secret field is masked and never echoed; a node's own action button opens a
// form too; an action whose `when` is false is not offered.

const manifest = `permissions: [commands:oxplow.work_item.create, commands:oxplow.work_item.delete]
commands:
  - name: secret.save
    summary: Files a wallaby named by the label, with a secret token that is never kept.
    entry: handlers/save.star
    unrecorded: [token]
    input_schema:
      type: object
      required: [label, token]
      properties:
        label: { type: string }
        token: { type: string, writeOnly: true }
`;

const handler = `def transform(x):
    return {"commands": [{"name": "oxplow.work_item.create", "input": {"title": "Wallaby " + x["input"]["label"]}}]}
`;

const pageYaml = `title: Action Wallaby
category: Data
loaders:
  rows:
    sql: SELECT uri, title, title NOT LIKE '%locked%' AS can_rename FROM v_work_item WHERE title LIKE 'Wallaby %' ORDER BY title
view:
  id: rows
  viz: table
  from: rows
  columns: [{ key: title, label: Wallaby }]
  empty: No wallabies.
  actions:
  - id: add
    label: Add a wallaby
    command: oxplow.work_item.create
    form: true
    fields: [title, description]
    input: { title: 'Wallaby ' }
  - id: keep-secret
    label: Add with a secret
    command: e2e_actions.secret.save
    form: true
    input: { label: '' }
  - id: copy
    label: Copy…
    command: oxplow.work_item.create
    row: true
    form: true
    fields: [title, description]
    input: { title: '{{row.title}} copy' }
  - id: rename
    label: Another…
    command: oxplow.work_item.create
    row: true
    form: true
    fields: [title]
    input: { title: '{{row.title}} again' }
    when: can_rename
  - id: remove
    label: Remove…
    command: oxplow.work_item.delete
    row: true
    input: { uri: '{{row.uri}}' }
`;

// A form node: its `secret` field is masked, and no loader holds it.
const formPage = `title: Form Wallaby
category: Data
loaders:
  rows:
    sql: SELECT '' AS label, uri, title FROM v_work_item WHERE title LIKE 'Wallaby %' ORDER BY title
view:
  stack:
  - id: made
    title: Made
    from: rows
    viz: table
    columns: [{ key: title, label: Wallaby }]
    empty: No wallabies.
  - id: keep
    title: Keep
    from: rows
    viz: form
    form:
      command: e2e_actions.secret.save
      fields:
      - label
      - { name: token, secret: true }
`;

async function openPage(page: Page, daemon: Parameters<typeof installExtension>[0], id = "wallaby") {
  await installExtension(daemon, "e2e-actions", {
    manifest,
    files: { "pages/wallaby.yaml": pageYaml, "pages/form.yaml": formPage, "handlers/save.star": handler },
  });
  await open(page, daemon, `oxplow:oxplow/page?page=e2e_actions/${id}`);
  return shown(page).getByTestId("page-host");
}

test("a row's menu opens a form, the form submits, and the page shows the result", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Wallaby one", thread: daemon.thread });
  const host = await openPage(page, daemon);
  const row = host.getByRole("row", { name: /Wallaby one/ });
  await row.click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-copy").click();
  // Two fields: a modal form, its field starting from the row.
  const form = page.getByTestId("viz-action-form");
  const title = form.getByTestId("viz-action-form-fields-title");
  await expect(title).toHaveValue("Wallaby one copy");
  await title.fill("Wallaby one copied");
  await form.getByTestId("viz-action-form-fields-submit").click();
  await expect(form).toHaveCount(0);
  await expect(host).toContainText("Wallaby one copied");
});

test("a form of one text field is a prompt strip, and Enter submits it", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Wallaby two", thread: daemon.thread });
  const host = await openPage(page, daemon);
  await host.getByRole("row", { name: /Wallaby two/ }).click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-rename").click();
  const field = page.getByTestId("viz-action-form-fields-title");
  await expect(field).toHaveValue("Wallaby two again");
  await field.fill("Wallaby two prompted");
  await field.press("Enter");
  await expect(field).toHaveCount(0);
  await expect(host).toContainText("Wallaby two prompted");
});

test("a command that asks is confirmed in a strip before it runs", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Wallaby three", thread: daemon.thread });
  const host = await openPage(page, daemon);
  const row = host.getByRole("row", { name: /Wallaby three/ });
  await row.click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-remove").click();
  // Nothing has run: the row is still there until the strip is confirmed.
  await expect(page.getByTestId("viz-action-confirm-run")).toBeVisible();
  await expect(row).toBeVisible();
  await page.getByTestId("viz-action-confirm-run").click();
  await expect(host.getByRole("row", { name: /Wallaby three/ })).toHaveCount(0);
});

test("an action whose `when` is false is not offered on that row", async ({ page, daemon }) => {
  await run(daemon, "oxplow.work_item.create", { title: "Wallaby locked", thread: daemon.thread });
  await run(daemon, "oxplow.work_item.create", { title: "Wallaby open", thread: daemon.thread });
  const host = await openPage(page, daemon);
  await host.getByRole("row", { name: /Wallaby locked/ }).click({ button: "right" });
  await expect(page.getByTestId("menu-item-viz-action-copy")).toBeVisible();
  await expect(page.getByTestId("menu-item-viz-action-rename")).not.toBeVisible();
  await page.keyboard.press("Escape");
  await host.getByRole("row", { name: /Wallaby open/ }).click({ button: "right" });
  await expect(page.getByTestId("menu-item-viz-action-rename")).toBeVisible();
});

test("a node's own action button opens a form, and a secret field is masked and never echoed", async ({ page, daemon }) => {
  const host = await openPage(page, daemon);
  await expect(host).toContainText("No wallabies.");
  await host.getByTestId("viz-action-add").click();
  const form = page.getByTestId("viz-action-form");
  await form.getByTestId("viz-action-form-fields-title").fill("Wallaby from the node");
  await form.getByTestId("viz-action-form-fields-submit").click();
  await expect(host).toContainText("Wallaby from the node");

  await host.getByTestId("viz-action-keep-secret").click();
  const secret = page.getByTestId("viz-action-form-fields-token");
  await expect(secret).toHaveAttribute("type", "password");
  await page.getByTestId("viz-action-form-fields-label").fill("kept");
  await secret.fill("s3cret-token-value");
  await page.getByTestId("viz-action-form-fields-submit").click();
  await expect(host).toContainText("Wallaby kept");
  await expect(page.locator("body")).not.toContainText("s3cret-token-value");
});

test("a form node's secret field is masked, and the page never shows it", async ({ page, daemon }) => {
  const host = await openPage(page, daemon, "form");
  const form = host.locator('[data-node="keep"]');
  const secret = form.getByTestId("viz-form-fields-token");
  await expect(secret).toHaveAttribute("type", "password");
  await form.getByTestId("viz-form-fields-label").fill("formed");
  await secret.fill("another-secret-value");
  await form.getByTestId("viz-form-fields-submit").click();
  await expect(host.locator('[data-node="made"]')).toContainText("Wallaby formed");
  await expect(page.locator("body")).not.toContainText("another-secret-value");
});
