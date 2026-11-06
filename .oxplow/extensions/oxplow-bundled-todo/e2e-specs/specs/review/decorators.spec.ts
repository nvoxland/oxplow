import { ipc, run, until } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// oxplow-bundled decorates an effort with its latest verdict: a chip
// in the effort page's header.

test("an accepted effort's page leads with the effort-verdict replacement", async ({ page, daemon }) => {
  const created = await run<{ result: { uri: string } }>(daemon, "oxplow.work_item.create", {
    title: "Wombat fix",
    state: "in_progress",
    thread: daemon.thread,
  });
  // The effort opens in the background once the item is in progress.
  let effort = "";
  await until("the item's effort to open", 30_000, async () => {
    const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
      sql: `SELECT uri FROM v_effort WHERE work_item = '${created.result.uri}'`,
    });
    effort = (out.rows[0]?.[0] as string | undefined) ?? "";
    return effort !== "";
  });
  expect(effort).toMatch(/^oxplow:oxplow\/effort\?id=eff\d+$/);
  await run(daemon, "oxplow.review.accept", { effort: new URL(effort).searchParams.get("id") });
  await page.goto("/");
  // The window answers a navigate once its stream is shown: a page open is the sign.
  await openFromLauncher(page, "Git Status");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
  // The effort's own page.
  await run(daemon, "oxplow.window.navigate", { uri: effort });
  // It leads with oxplow-bundled's verdict: no tests ran in this effort.
  await expect(page.getByTestId("replacement-effort.verdict")).toContainText("Tests: none ran");
});
