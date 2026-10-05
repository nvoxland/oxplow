import { ipc } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

type Extension = { name: string; errors: string[] };
type Program = { kind: string; name: string; approved: boolean };

test("the test extension loads clean", async ({ daemon }) => {
  const extensions = await ipc<Extension[]>(daemon, "list_extensions");
  expect(extensions.find((e) => e.name === "e2e")?.errors).toEqual([]);
});

test("a person approves the extension's provider and effect on Settings", async ({ fresh }) => {
  const { page, daemon } = fresh;
  await page.goto("/");
  await openFromLauncher(page, "Settings");
  // The index jumps to a section (tsk1040).
  await expect(page.getByTestId("settings-index-settings-data-programs")).toBeVisible();
  const notApproved = page.locator('[data-testid^="integration-needs-approval-"]');
  await expect(notApproved.first()).toBeVisible();
  for (const key of ["provider:e2e/fake", "effect:e2e/note-created"]) {
    const approve = page.getByTestId(`program-approve-${key}`);
    await expect(approve).toBeEnabled();
    await approve.click();
    await expect(approve).toHaveCount(0);
  }
  // Integrations hears the approval: no stale "isn't approved" (tsk1040).
  await expect(notApproved).toHaveCount(0);
  const programs = await ipc<Program[]>(daemon, "list_project_programs");
  const approved = (kind: string, name: string) => programs.find((p) => p.kind === kind && p.name === name)?.approved;
  expect(approved("provider", "e2e/fake")).toBe(true);
  expect(approved("effect", "e2e/note-created")).toBe(true);
});
