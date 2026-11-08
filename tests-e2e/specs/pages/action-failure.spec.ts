import { ipc, type Daemon } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { inFrame, open, shown } from "../../support/ui.js";

// A person's command that fails (`.context/usability.md` "A person's command
// that fails"): it is recorded once, as a failed operation, and shows beside
// the control that ran it, with Details; the toast is only for one whose
// control is gone. The suite's `e2e.fail.now` always refuses.

/** The failed operations recorded (each test has its own project). */
async function recorded(daemon: Daemon): Promise<number> {
  const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", { sql: "SELECT count(*) FROM v_op_error" });
  return Number(out.rows[0]?.[0] ?? 0);
}

test("a page action that fails says so beside its button, recorded once, with no toast", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/failing");
  await shown(page).getByTestId("action-page-action-boom").click();
  const error = shown(page).getByTestId("action-error-page-action-boom");
  await expect(error).toContainText("the suite asked for a failure");
  await expect.poll(() => recorded(daemon)).toBe(1);
  // The bell counts it; the toast stays away while the button is on screen.
  await expect(page.getByTestId("dock-alerts-badge")).toHaveText("1");
  await expect(page.getByTestId("undo-toast")).not.toBeVisible();
  // Details opens where it is recorded.
  await error.getByTestId("inline-error-details").click();
  await expect(shown(page).getByRole("heading", { name: "Failed operations" })).toBeVisible();
});

test("a failure whose page closed before it landed is the toast's, offering Review", async ({ page, daemon }) => {
  const uri = "oxplow:oxplow/page?page=e2e/failing";
  await open(page, daemon, uri);
  await shown(page).getByTestId("action-page-action-boom").click();
  await page.getByTestId(`center-tab-close-${uri}`).click();
  const toast = page.getByTestId("undo-toast").filter({ hasText: "Boom" });
  await expect(toast).toBeVisible();
  await expect(toast.getByTestId("undo-toast-undo")).toHaveText("Review");
});

test("a component's invoke that fails shows under its frame and is recorded, whatever the component draws", async ({ page, daemon }) => {
  await open(page, daemon, "oxplow:oxplow/page?page=e2e/boom");
  const frame = await inFrame(shown(page));
  await frame.locator("#boom").click();
  await expect(shown(page).getByTestId("custom-component-refused")).toContainText("the suite asked for a failure");
  await expect.poll(() => recorded(daemon)).toBe(1);
});
