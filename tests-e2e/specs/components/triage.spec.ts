import { cpSync, mkdirSync, writeFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type { Page } from "@playwright/test";

import { approveCollector, approveProgram, installExample, ipc, run, until, type Daemon } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The triage example (`examples/extensions/triage`): its `todos` collector
// sorts the code's TODO comments with a model, and a TODO is decided from
// the board's card form or from the `todos` lens's `Decide…` row action —
// in Chromium and WebKit. The model is the suite extension's scripted
// provider `e2e/sorter`, whose host call goes to an echo server here: the
// script decides, offline.

/** A server that answers each POST with the body it got. */
async function echo(): Promise<{ url: string; close(): Promise<void> }> {
  const server: Server = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(Buffer.concat(chunks));
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = (server.address() as AddressInfo).port;
  return { url: `http://127.0.0.1:${port}`, close: () => new Promise((r) => server.close(() => r())) };
}

/** Put the example into `daemon`'s project and deploy it. */
async function install(daemon: Daemon) {
  await installExample(daemon, "triage");
}

async function facts(daemon: Daemon): Promise<unknown[][]> {
  const out = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
    sql: "SELECT path, line, kind FROM (SELECT path, line, rule AS kind FROM metric_findings('triage.todos')) ORDER BY path, line",
  });
  return out.rows;
}

/** Reload once the page has no request in flight: WebKit reports a fetch
 *  the reload cancels as an error the page threw. */
async function reload(page: Page) {
  await page.waitForLoadState("networkidle");
  await page.reload();
}

/** What was decided, by `<path>:<line>`, as the extension reads it back. */
async function decisions(daemon: Daemon): Promise<Record<string, string>> {
  const out = await run<{ result: { result: { todos: Array<{ path: string; line: number; decision: string | null }> } } }>(
    daemon,
    "triage.todos.decisions",
    {},
  );
  const decided: Record<string, string> = {};
  for (const t of out.result.result.todos) if (t.decision) decided[`${t.path}:${t.line}`] = t.decision;
  return decided;
}

test("TODOs are sorted by the scripted model, and a decision from the board or the lens survives a reload", async ({ fresh }) => {
  const { page, daemon } = fresh;
  const model = await echo();
  try {
    await install(daemon);
    // The model: the suite extension's scripted provider, at the echo server.
    await approveProgram(daemon, "ai-provider", "e2e/sorter");
    await ipc(daemon, "save_ai_provider", { provider: { id: "sorter", kind: "sorter", baseUrl: model.url }, key: null });
    await ipc(daemon, "set_ai_role", { role: "decide", binding: { provider: "sorter", model: "sorter" } });
    await until("the todos collector to be listed", 60_000, async () => {
      const collectors = await ipc<Array<{ owner: string; spec: { id: string } }>>(daemon, "list_collectors");
      return collectors.some((c) => c.owner === "triage" && c.spec.id === "todos");
    });
    await approveCollector(daemon, "triage", "todos");

    mkdirSync(join(daemon.project, "src"), { recursive: true });
    writeFileSync(
      join(daemon.project, "src", "lib.rs"),
      "// TODO: fix the bug in the parser\nfn a() {}\n// TODO: tidy the module\nfn b() {}\n",
    );
    await run(daemon, "oxplow.collector.sync", { collector: "collector:triage/todos" });
    await until("the TODOs to be sorted", 60_000, async () => (await facts(daemon)).length === 2);
    expect(await facts(daemon)).toEqual([
      ["src/lib.rs", 1, "bug"],
      ["src/lib.rs", 3, "cleanup"],
    ]);
    await page.goto("/");
    await openFromLauncher(page, "TODO Board");
    const board = page.getByTestId("center-tab-body-lens:triage/todo-board");
    const frame = board.frameLocator('[data-testid="custom-component-frame"]');
    const card = (line: number) => frame.locator(".card", { hasText: `src/lib.rs:${line}` });
    await expect(frame.locator(".column", { hasText: "bug" }).locator(".card")).toHaveCount(1);
    await expect(card(3)).toBeVisible();
    // Until the board is approved it shows the TODOs and says so.
    await expect(frame.locator("#status")).toContainText("Approve");
    await approveProgram(daemon, "component", "triage/todo-board");
    await reload(page);
    await openFromLauncher(page, "TODO Board");
    await expect(frame.locator("#status")).toHaveText("");

    // A decision from the board's card form.
    await card(1).locator("select[name=decision]").selectOption("fix-now");
    await card(1).getByRole("button", { name: "Save" }).click();
    await until("the board's decision to be stored", 15_000, async () => (await decisions(daemon))["src/lib.rs:1"] === "fix-now");

    // A decision from the lens's `Decide…` row action, in its slideover.
    await openFromLauncher(page, "TODOs");
    const lens = page.getByTestId("center-tab-body-lens:triage/todos");
    await lens.getByRole("row").filter({ hasText: "cleanup" }).locator("td").first().click({ button: "right" });
    await page.getByTestId("menu-item-lens-action-decide").click();
    const form = page.getByTestId("lens-action-form");
    await form.getByTestId("lens-action-form-fields-decision").selectOption("wontfix");
    await form.getByTestId("lens-action-form-fields-submit").click();
    await expect(form).toHaveCount(0);
    await until("the lens's decision to be stored", 15_000, async () => (await decisions(daemon))["src/lib.rs:3"] === "wontfix");

    // Both survive a reload: the board shows them again.
    await reload(page);
    await openFromLauncher(page, "TODO Board");
    await expect(card(1).locator("select[name=decision]")).toHaveValue("fix-now");
    await expect(card(3).locator("select[name=decision]")).toHaveValue("wontfix");
    expect(await decisions(daemon)).toEqual({ "src/lib.rs:1": "fix-now", "src/lib.rs:3": "wontfix" });
  } finally {
    await model.close();
  }
});
