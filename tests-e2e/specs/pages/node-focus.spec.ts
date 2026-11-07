import { expect, test } from "../../support/fixtures.js";
import { run } from "../../support/daemon.js";
import { open, shown } from "../../support/ui.js";

// A tall page: sixty rows above the node a link lands on.
const spec = `title: Focus Wombat
loaders:
  rows:
    sql: WITH RECURSIVE c(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c WHERE n < 60) SELECT n FROM c
  one:
    sql: SELECT 1 AS n
view:
  stack:
    - { id: top, title: Top, viz: table, from: rows }
    - { id: target, title: Target, viz: number, from: one }
`;

test("opening a page URI with #node= scrolls to the node and marks it", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/home");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
  const made = await run<{ result: { uri: string; opened: boolean; why?: string } }>(daemon, "oxplow.page.show", { spec });
  expect(made.result.opened, made.result.why).toBe(true);
  await run(daemon, "oxplow.window.navigate", { uri: `${made.result.uri}#node=target` });
  const target = shown(page).getByTestId("page-file").locator('[data-node="target"]');
  await expect(target).toHaveAttribute("data-focused", "true");
  await expect(target).toBeInViewport();
  // The mark is brief: the node stays, the highlight goes.
  await expect(target).not.toHaveAttribute("data-focused", "true");
});
