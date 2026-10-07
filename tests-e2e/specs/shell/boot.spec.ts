import { expect, test } from "../../support/fixtures.js";

test("the app boots against its daemon: the shell renders and no call fails", async ({ page }) => {
  const failed: string[] = [];
  page.on("requestfailed", (r) => failed.push(`${r.method()} ${r.url()}: ${r.failure()?.errorText}`));
  // The daemon answers every call 200 with an envelope: a failed call is
  // `status: "error"` inside it.
  const replies: Promise<unknown>[] = [];
  page.on("response", (r) => {
    if (!r.url().includes("/ipc/")) return;
    replies.push(
      r.json().then(
        (reply: { status?: string; error?: unknown }) => {
          if (!r.ok() || reply.status !== "ok") failed.push(`${r.status()} ${r.url()}: ${JSON.stringify(reply.error)}`);
        },
        (e: unknown) => failed.push(`${r.status()} ${r.url()}: not JSON (${String(e)})`),
      ),
    );
  });
  await page.goto("/");
  await expect(page.getByTestId("rail-hud")).toBeVisible();
  await expect(page.getByTestId("title-bar")).toBeVisible();
  await expect(page.getByTestId("page-agent")).toBeVisible();
  expect(replies.length, "the app made its calls").toBeGreaterThan(0);
  await Promise.all(replies);
  expect(failed).toEqual([]);
});
