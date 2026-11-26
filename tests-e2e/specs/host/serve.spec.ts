// The host serves the built UI and forwards a project's calls: a browser
// that only knows the host's address pairs once and gets a working window.
// No token is in a page, and nothing it loads comes from another origin.
import { expect, test, type Page } from "@playwright/test";

import { startHost, type Host } from "../../support/host.js";
import { loadScreen } from "../../support/ui.js";

let host: Host;
test.beforeAll(async () => {
  host = await startHost();
});
test.afterAll(async () => {
  await host?.stop();
});

/** Pair `page` at the host's home with a code the owner made. */
async function pair(page: Page, name: string) {
  await loadScreen(page, `${host.base}/`, "pair-screen");
  await page.getByTestId("pair-code").fill(await host.pairingCode());
  await page.getByTestId("pair-name").fill(name);
  await page.getByTestId("pair-name").press("Enter");
  await expect(page.getByTestId("host-home")).toBeVisible();
}

test("a device pairs at the host's home, opens a project and its window is ready", async ({ page }) => {
  test.setTimeout(240_000);
  const id = host.addProject("alpha");

  const origins = new Set<string>();
  const failed: string[] = [];
  page.on("request", (r) => origins.add(new URL(r.url()).origin));
  page.on("requestfailed", (r) => failed.push(`${r.method()} ${r.url()}: ${r.failure()?.errorText}`));
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));

  // Nothing of the projects is on the home page before pairing.
  const root = await (await fetch(host.base)).text();
  expect(root).not.toContain(id);
  await pair(page, "Tab S9");
  // Open holds it for this device: its daemon starts.
  await page.getByTestId(`host-project-open-${id}`).click();
  await page.waitForURL(`${host.base}/p/${id}/`);
  await expect(page.getByTestId("rail-hud")).toBeVisible({ timeout: 180_000 });
  await expect(page.getByTestId("title-bar")).toBeVisible();

  const context = await page.evaluate(() => (window as unknown as { __OXPLOW__: Record<string, unknown> }).__OXPLOW__);
  expect(context.kind).toBe("project");
  expect(context.base).toBe(`${host.base}/p/${id}`);
  expect(context.host).toBe(host.base);
  expect(context.token).toBeNull();
  // The host knows this device by its record.
  const [device] = await host.devices();
  expect(device!.name).toBe("Tab S9");
  await expect.poll(async () => (await host.status()).connected.map((c) => c.name)).toContain("Tab S9");
  // Everything came from the host's one origin, and nothing failed.
  expect([...origins].filter((o) => o !== host.base && !o.startsWith("data:") && !o.startsWith("blob:"))).toEqual([]);
  expect(failed).toEqual([]);
  expect(errors).toEqual([]);
});

test("a device its owner removes is asked to pair again", async ({ page }) => {
  test.setTimeout(240_000);
  const id = host.addProject("gamma");
  await pair(page, "Phone");
  await page.getByTestId(`host-project-open-${id}`).click();
  await expect(page.getByTestId("rail-hud")).toBeVisible({ timeout: 180_000 });
  const phone = (await host.devices()).find((d) => d.name === "Phone")!;
  await host.removeDevice(phone.id);
  await expect(page.getByTestId("pair-screen")).toBeVisible();
});

test("a project that is not held has no daemon to reach", async () => {
  const id = host.addProject("beta");
  const res = await fetch(`${host.base}/p/${id}/health`);
  expect(res.status).toBe(404);
  expect(((await res.json()) as { state: string }).state).toBe("stopped");
});
