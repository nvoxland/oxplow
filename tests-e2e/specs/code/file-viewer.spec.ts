import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// A file of a filesystem other than the working tree, or at a past
// revision, is the Text view's: read-only, no Save, and the Editor is
// never offered for it. A working-tree text file is the Editor's alone:
// View as has no other mode to offer. A binary file is viewed, never
// edited: the Text view's too (an image or a PDF: the Media page's).

type Page = import("@playwright/test").Page;

async function ready(page: Page) {
  await page.goto("/");
  await openFromLauncher(page, "Suite Home");
  await expect(page.getByTestId("rail-hud")).toHaveAttribute("aria-busy", "false");
}

const navigate = (daemon: Parameters<typeof run>[0], uri: string) => run(daemon, "oxplow.window.navigate", { uri });

test("a file on the running extensions opens read-only in the Text view", async ({ page, daemon }) => {
  await ready(page);
  const uri = "oxplow:oxplow/file?path=oxplow-foundation/extension.yaml&filesystem=oxplow:extensions";
  await navigate(daemon, uri);
  const code = page.getByTestId("viz-code").filter({ visible: true });
  await expect(code).toContainText("oxplow-foundation");
  await expect(page.getByTestId("viz-editor")).toHaveCount(0);
  // No Save: the tab has no dirty mark and File → Save has nothing to write.
  await expect(page.getByTestId(`center-tab-dirty-${uri}`)).toHaveCount(0);
  await expect(page.getByTestId("viz-code-path").filter({ visible: true })).toHaveText("oxplow-foundation/extension.yaml");
  await expect(page.getByTestId("viz-code-file").filter({ visible: true })).toContainText("@oxplow:extensions");
  // A page that fills, with no filter, selection or rail node, has no
  // Details card.
  await expect(page.getByTestId("page-rail").filter({ visible: true })).toHaveCount(0);
});

test("a scratch file on oxplow:temp opens read-only in the Text view", async ({ page, daemon }) => {
  const temp = join(daemon.project, ".oxplow", "runtime", "temp");
  mkdirSync(temp, { recursive: true });
  writeFileSync(join(temp, "scratch.txt"), "scratch notes\n");
  await ready(page);
  await navigate(daemon, "oxplow:oxplow/file?path=scratch.txt&filesystem=oxplow:temp");
  await expect(page.getByTestId("viz-code").filter({ visible: true })).toContainText("scratch notes");
  await expect(page.getByTestId("viz-editor")).toHaveCount(0);
});

test("a binary file shows what it is instead of its bytes, in the Text view and never the Editor", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "blob.bin"), Buffer.from([0, 1, 2, 3, 0, 255]));
  await ready(page);
  // A binary working-tree file has nothing to edit: the Text view opens it.
  await navigate(daemon, "oxplow:oxplow/file?path=blob.bin");
  await expect(page.getByTestId("viz-code").filter({ visible: true })).toContainText("binary file, 6 bytes");
  await expect(page.getByTestId("viz-editor")).toHaveCount(0);
  await navigate(daemon, "oxplow:oxplow/file?path=blob.bin&rev=working");
  await expect(page.getByTestId("viz-code").filter({ visible: true })).toContainText("binary file, 6 bytes");
});

test("a working-tree file is the Editor's alone: View as has no other mode", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "koala.txt"), "koala\n");
  await ready(page);
  await navigate(daemon, "oxplow:oxplow/file?path=koala.txt");
  await expect(page.getByTestId("viz-editor").filter({ visible: true })).toBeVisible();
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await expect(page.getByTestId("menu-item-action-page-about")).toBeVisible();
  await expect(page.getByTestId("menu-item-page-open-with")).toHaveCount(0);
});

// The Text view's body is the file, so its viz draws no Open file. A
// local file at a past revision has a working-tree version: the page's ☰
// offers Open current version, which opens it in the Editor. An extension's
// file has neither.
test("a local file at a past revision has no inline Open file, and its ☰ opens the current version in the Editor", async ({ page, daemon }) => {
  writeFileSync(join(daemon.project, "wombat.txt"), "wombat\n");
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "user.name=e2e", "-c", "user.email=e2e@example.com", ...args], { cwd: daemon.project });
  git("add", "wombat.txt");
  git("commit", "-q", "-m", "Add wombat");
  await ready(page);
  await navigate(daemon, "oxplow:oxplow/file?path=wombat.txt&rev=git:HEAD");
  await expect(page.getByTestId("viz-code").filter({ visible: true })).toContainText("wombat");
  await expect(page.getByTestId("viz-code-open")).toHaveCount(0);
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await page.getByTestId("menu-item-action-page-action-open-current").click();
  await expect(page.getByTestId("viz-editor").filter({ visible: true })).toContainText("wombat");
});

test("a file on the running extensions has neither Open file nor Open current version", async ({ page, daemon }) => {
  await ready(page);
  await navigate(daemon, "oxplow:oxplow/file?path=oxplow-foundation/extension.yaml&filesystem=oxplow:extensions");
  await expect(page.getByTestId("viz-code").filter({ visible: true })).toContainText("oxplow-foundation");
  await expect(page.getByTestId("viz-code-open")).toHaveCount(0);
  await page.getByTestId("page-nav-menu").filter({ visible: true }).click();
  await expect(page.getByTestId("menu-item-action-page-about")).toBeVisible();
  await expect(page.getByTestId("menu-item-action-page-action-open-current")).toHaveCount(0);
});
