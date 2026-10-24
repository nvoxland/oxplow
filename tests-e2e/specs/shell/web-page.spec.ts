import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The Web Page tab is a foundation lens (`viz: embed`): an http(s) link
// opens it, labelled by the page's host and path; the page itself opens in
// the sandboxed window, so the tab is its status panel.
test("an http link opens the Web Page tab, labelled by its host", async ({ page, daemon }) => {
  await page.goto("/");
  await openFromLauncher(page, "Tasks");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
  const url = "https://example.com/docs/start";
  await run(daemon, "oxplow.window.navigate", { uri: url, new_tab: true });
  const tab = page.getByTestId(`center-tab-${url}`);
  await expect(tab).toBeVisible();
  await expect(tab).toContainText("example.com/docs/start");
  const panel = page.getByTestId("lens-embed");
  await expect(panel).toContainText("example.com");
  await expect(panel.getByTestId("lens-embed-copy")).toBeVisible();
  await expect(panel.getByTestId("lens-embed-browser")).toBeVisible();
});
