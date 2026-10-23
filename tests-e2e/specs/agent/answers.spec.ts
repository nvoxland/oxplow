import { expect, test } from "../../support/fixtures.js";
import { ipc } from "../../support/daemon.js";

// What an agent shows (`oxplow.lens.show`) is its thread's: it opens as a
// tab of the thread, behind what the person is on, and the thread's
// Answers panel lists it. Its page runs it live with Keep This.

test("a shown answer opens as a tab of its thread, listed by the Answers panel, and is kept from its page", async ({ page, daemon }) => {
  await page.goto("/");
  const panel = page.getByTestId("rail-section-ext:oxplow-foundation/answers");
  await expect(panel).toBeVisible();
  const threads = await ipc<{ rows: [string][] }>(daemon, "query_sql", { sql: "SELECT uri FROM v_thread ORDER BY id LIMIT 1" });
  const thread = threads.rows[0]![0].replace(/^.*id=/, "");
  const shown = await ipc<{ result: { answer: number } }>(daemon, "run_command", {
    id: "oxplow.lens.show",
    input: { thread, spec: { title: "Kiwi Count", query: "SELECT 42 AS kiwis", viz: "number" } },
    confirmed: false,
  });
  const uri = `oxplow:oxplow/answer?id=${shown.result.answer}`;
  const tab = page.getByTestId(`center-tab-${uri}`);
  await expect(tab).toContainText("Kiwi Count");
  await expect(panel).toContainText("Kiwi Count");
  await tab.click();
  const body = page.getByTestId("lens-answer-body").filter({ visible: true });
  await expect(body).toContainText("42");
  await page.getByTestId("lens-answer-keep").filter({ visible: true }).click();
  await page.getByTestId("lens-answer-slug").fill("kiwi-count");
  await page.getByTestId("lens-answer-slug").press("Enter");
  await expect(page.getByTestId("lens-answer-lens").filter({ visible: true })).toContainText("kiwi-count");
});
