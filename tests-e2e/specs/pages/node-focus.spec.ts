import { expect, test } from "../../support/fixtures.js";
import { run } from "../../support/daemon.js";
import { openFromLauncher } from "../../support/ui.js";

// A tall page: sixty rows above the node a link lands on.
const spec = `title: Focus Wombat
loaders:
  rows:
    sql: WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 60) SELECT n FROM c
  one:
    sql: SELECT 1 AS n
view:
  stack:
    - { id: top, viz: table, from: rows }
    - { id: target, viz: number, from: one }
`;

test("opening a page URI with #node= scrolls to the node and marks it", async ({ page, daemon }) => {
  await page.goto("/");
  await openFromLauncher(page, "Tasks");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
  const shown = await run<{ result: { uri: string; opened: boolean; why?: string } }>(daemon, "oxplow.page.show", { spec });
  expect(shown.result.opened, shown.result.why).toBe(true);
  await run(daemon, "oxplow.window.navigate", { uri: `${shown.result.uri}#node=target` });
  const target = page.getByTestId("page-file").filter({ visible: true }).locator('[data-node="target"]');
  await expect(target).toHaveAttribute("data-focused", "true");
  await expect(target).toBeInViewport();
  // The mark is brief: the node stays, the highlight goes.
  await expect(target).not.toHaveAttribute("data-focused", "true");
});
