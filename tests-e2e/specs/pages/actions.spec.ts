import type { Page } from "@playwright/test";

import { fileTask, ipc } from "../../support/daemon.js";
import { installExtension } from "../../support/pages.js";
import { expect, test } from "../../support/fixtures.js";
import { open, shown } from "../../support/ui.js";

// What any page's actions do in the shell, over one small extension: a row's
// right-click menu opens a form (a modal, or a prompt strip when it asks for
// one field) and submits it; a command that asks is confirmed in a strip; a
// secret field is masked and never echoed; a node's own action button opens a
// form too; an action whose `when` is false is not offered; an action with
// options is a submenu of them, the row's own checked.

const manifest = `requires: [oxplow-bundled, oxtask]
permissions: [sql.read, commands:oxtask.task.create, commands:oxtask.task.delete, commands:oxplow.bookmark.set]
commands:
  - name: secret.save
    summary: Notes a wallaby named by the label, with a secret token that is never kept.
    entry: handlers/save.star
    needs: [sql.read]
    unrecorded: [token]
    input_schema:
      type: object
      required: [label, token]
      properties:
        label: { type: string }
        token: { type: string, writeOnly: true }
`;

// A task store's own commands write outside the transaction, so a command of
// an extension can't compose one: this one notes the wallaby as a bookmark.
const handler = `def transform(x):
    stream = scope("sql.read", {"sql": "SELECT 'str' || id AS stream FROM v_stream LIMIT 1", "params": {}})[0]["stream"]
    return {"commands": [{"name": "oxplow.bookmark.set", "input": {"uri": "oxplow:oxplow/file?path=wallabies%2F" + x["input"]["label"], "page_kind": "file", "label": "Wallaby " + x["input"]["label"], "scope": "stream", "stream": stream}}]}
`;

const pageYaml = `title: Action Wallaby
category: Data
loaders:
  rows:
    sql: SELECT uri, native_id AS id, title, stage, title NOT LIKE '%locked%' AS can_rename FROM v_oxplow_task WHERE title LIKE 'Wallaby %'
      UNION ALL SELECT uri, NULL, label, NULL, 1 FROM v_bookmark WHERE label LIKE 'Wallaby %'
      ORDER BY title
  marks:
    sql: SELECT 'ready' AS value, 'Ready' AS label UNION ALL SELECT 'in_progress', 'Started' UNION ALL SELECT 'done', 'Finished'
view:
  id: rows
  viz: table
  from: rows
  columns: [{ key: title, label: Wallaby }]
  empty: No wallabies.
  actions:
  - id: add
    label: Add a wallaby
    command: oxtask.task.create
    form: true
    fields: [title, body]
    input: { title: 'Wallaby ' }
    then: stay
  - id: keep-secret
    label: Add with a secret
    command: e2e_actions.secret.save
    form: true
    input: { label: '' }
  - id: copy
    label: Copy…
    command: oxtask.task.create
    row: true
    form: true
    fields: [title, body]
    input: { title: '{{row.title}} copy' }
  - id: rename
    label: Another…
    command: oxtask.task.create
    row: true
    form: true
    fields: [title]
    input: { title: '{{row.title}} again' }
    when: can_rename
  - id: mark
    label: Mark
    command: oxtask.task.move
    row: true
    options_from: marks
    current: stage
    input: { id: '{{row.id}}', stage: '{{option.value}}' }
  - id: remove
    label: Remove…
    command: oxtask.task.delete
    row: true
    input: { id: '{{row.id}}' }
`;

// A form node: its `secret` field is masked, and no loader holds it.
const formPage = `title: Form Wallaby
category: Data
loaders:
  rows:
    sql: SELECT '' AS label, uri, title FROM v_oxplow_task WHERE title LIKE 'Wallaby %'
      UNION ALL SELECT '', uri, label FROM v_bookmark WHERE label LIKE 'Wallaby %'
      ORDER BY title
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
  await fileTask(daemon, "Wallaby one");
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
  await fileTask(daemon, "Wallaby two");
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
  await fileTask(daemon, "Wallaby three");
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
  await fileTask(daemon, "Wallaby locked");
  await fileTask(daemon, "Wallaby open");
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

test("an action with options is a submenu of them, the row's own checked, and the one picked runs", async ({ page, daemon }) => {
  await fileTask(daemon, "Wallaby marked");
  const host = await openPage(page, daemon);
  await host.getByRole("row", { name: /Wallaby marked/ }).click({ button: "right" });
  await page.getByTestId("menu-item-viz-action-mark").click();
  // A new item is in its list's todo stage: ready, checked and not offered.
  const own = page.getByTestId("menu-item-viz-action-mark-ready");
  await expect(own).toContainText("✓");
  await expect(own).toBeDisabled();
  await page.getByTestId("menu-item-viz-action-mark-done").click();
  await expect
    .poll(async () => {
      const r = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT stage FROM v_oxplow_task WHERE title = 'Wallaby marked'" });
      return r.rows[0]?.[0];
    })
    .toBe("done");
});
