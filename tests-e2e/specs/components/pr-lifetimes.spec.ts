import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import type { Page } from "@playwright/test";

import { approveCollector, approveProgram, ipc, run, until, type Daemon } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The github example's PR Lifetimes (P11, tsk962): the custom component
// that made `custom_components` stable, in its sandboxed frame — in
// Chromium and WebKit, the engines oxplow's browser mode and its macOS
// window run. Every suite daemon has the example, its sync printing the
// suite's two pull requests (#12 merged, #13 open).

/** Approve the example's sync as it is now and run it as a person, until
 *  `count` pull requests are collected. Its approval covers its
 *  extension's files, so a changed `prs.json` is approved again. */
async function collect(daemon: Daemon, count: number) {
  await approveCollector(daemon, "github", "prs");
  await run(daemon, "oxplow.collector.sync", { owner: "github", id: "prs" });
  await until(`${count} pull requests to be collected`, 30_000, async () => {
    const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT count(*) FROM v_github_pr" });
    return out.rows[0]?.[0] === count;
  });
}

/** Collect the pull requests (`count` of them, `bars` drawable) and open
 *  the lens. */
async function openLifetimes(page: Page, daemon: Daemon, count = 2, bars = 2) {
  await collect(daemon, count);
  await page.goto("/");
  await openFromLauncher(page, "PR Lifetimes");
  const frame = page.frameLocator('[data-testid="custom-component-frame"]');
  await expect(frame.locator(".bar")).toHaveCount(bars);
  return frame;
}

/** Add `prs` to what the example's sync prints. */
function addPrs(daemon: Daemon, prs: Array<Record<string, unknown>>) {
  const path = join(daemon.project, "oxplow", "extensions", "github", "prs.json");
  const printed = JSON.parse(readFileSync(path, "utf8")) as { entities: { pr: unknown[] } };
  printed.entities.pr.push(...prs);
  writeFileSync(path, JSON.stringify(printed));
}

const pr = (number: number, opened_at: string) => ({
  number,
  title: `PR ${number}`,
  body: "",
  state: "open",
  author: "octocat",
  head_branch: `b${number}`,
  draft: false,
  opened_at,
  merged_at: null,
  url: `https://github.com/o/r/pull/${number}`,
});

// tsk1008: a date that can't be read leaves that pull request out — said,
// not every bar blanked — and a selected filter holds when the host runs
// the lens again (its models changed), rather than redrawing everything.
test("a bad date is left out, and a filter holds across the host's re-run", async ({ fresh }) => {
  const { page, daemon } = fresh;
  addPrs(daemon, [pr(11, "not a date")]);
  const frame = await openLifetimes(page, daemon, 3, 2);
  await expect(frame.locator("#note")).toContainText("1 pull request has a date that can't be read");
  for (const rect of await frame.locator(".bar rect").all()) {
    expect(Number.isFinite(Number(await rect.getAttribute("x")))).toBe(true);
  }
  await frame.locator('[data-state="open"]').click();
  await expect(frame.locator(".bar")).toHaveCount(1);
  addPrs(daemon, [pr(14, "2026-09-30T00:00:00Z")]);
  await collect(daemon, 4);
  // The host's re-run, with the filter kept: the open ones, #13 and #14.
  await expect(frame.locator(".bar")).toHaveCount(2);
  await expect(frame.locator('.bar[data-ref="github_pr:14"]')).toHaveCount(1);
  await expect(frame.locator('.bar[data-ref="github_pr:12"]')).toHaveCount(0);
});

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
