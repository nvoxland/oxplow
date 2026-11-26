// The host serves the built UI and forwards a project's calls: a browser
// that only knows the host's address gets a working window. No daemon token
// reaches the page, and nothing it loads comes from another origin.
import { expect, test } from "@playwright/test";

import { startHost, type Host } from "../../support/host.js";
import { load } from "../../support/ui.js";

let host: Host;
test.beforeAll(async () => {
  host = await startHost();
});
test.afterAll(async () => {
  await host?.stop();
});

test("a project's page loads through the host and its window is ready", async ({ page }) => {
  test.setTimeout(240_000);
  const id = host.addProject("alpha");
  await host.hold(id);

  const origins = new Set<string>();
  const failed: string[] = [];
  page.on("request", (r) => origins.add(new URL(r.url()).origin));
  page.on("requestfailed", (r) => failed.push(`${r.method()} ${r.url()}: ${r.failure()?.errorText}`));
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));

  // The root lists what is held, with a link to each project's page.
  const root = await (await fetch(host.base)).text();
  expect(root).toContain(`href="/p/${id}/"`);
  await load(page, `${host.base}/p/${id}/`);
  await expect(page.getByTestId("rail-hud")).toBeVisible();
  await expect(page.getByTestId("title-bar")).toBeVisible();

  const context = await page.evaluate(() => (window as unknown as { __OXPLOW__: Record<string, unknown> }).__OXPLOW__);
  expect(context.kind).toBe("project");
  expect(context.base).toBe(`${host.base}/p/${id}`);
  expect(context.project).toBe(id);
  // Everything came from the host's one origin, and nothing failed.
  expect([...origins].filter((o) => o !== host.base && !o.startsWith("data:") && !o.startsWith("blob:"))).toEqual([]);
  expect(failed).toEqual([]);
  expect(errors).toEqual([]);
});

test("a project that is not held has no daemon to reach", async () => {
  const id = host.addProject("beta");
  const res = await fetch(`${host.base}/p/${id}/health`);
  expect(res.status).toBe(404);
  expect(((await res.json()) as { state: string }).state).toBe("stopped");
});
