import { cpSync, mkdirSync, writeFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type { Page } from "@playwright/test";

import { installExample, ipc, run, until, type Daemon } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { openFromLauncher } from "../../support/ui.js";

// The triage example (`examples/extensions/triage`): its `todos` collector
// sorts the code's TODO comments with a model, and a TODO is decided from
// the board's card form or from the `todos` page's `Decide…` row action —
// in Chromium and WebKit. The model is a TypeSafe (`typesafe` kind)
// provider at a stand-in for Jev's API here, which decides offline.

type JevQuestion = { type: string; criteria?: Record<string, null> };

/** A stand-in for Jev's `/v1/systemone`: each choice question is answered
 *  "bug" when the text mentions a bug (and "bug" is an option), else
 *  "cleanup", else the first option. */
async function jev(): Promise<{ url: string; close(): Promise<void> }> {
  const server: Server = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => {
      const body = JSON.parse(Buffer.concat(chunks).toString()) as {
        state: string;
        questions: Record<string, JevQuestion>;
      };
      const text = body.state.toLowerCase();
      const answers: Record<string, { type: string; choice: string }> = {};
      for (const [name, q] of Object.entries(body.questions)) {
        const options = Object.keys(q.criteria ?? {});
        let pick = options[0] ?? "";
        if (text.includes("bug") && options.includes("bug")) pick = "bug";
        else if (options.includes("cleanup")) pick = "cleanup";
        answers[name] = { type: "choice", choice: pick };
      }
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ answers, usage: { input_tokens: 1, output_tokens: 1 } }));
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

test("TODOs are sorted by the model, and a decision from the board or the list page survives a reload", async ({ fresh }) => {
  const { page, daemon } = fresh;
  const model = await jev();
  try {
    await install(daemon);
    // The model: a TypeSafe provider at the stand-in for Jev.
    await run(daemon, "oxplow.ai.set_provider", { id: "sorter", kind: "typesafe", base_url: model.url });
    await run(daemon, "oxplow.ai.set_role", { role: "decide", provider: "sorter", model: "jev" });
    await until("the todos collector to be listed", 20_000, async () => {
      const collectors = await ipc<{ rows: unknown[][] }>(daemon, "query_sql", {
        sql: "SELECT 1 FROM v_collector WHERE extension = 'triage' AND id = 'todos'",
      });
      return collectors.rows.length > 0;
    });

    mkdirSync(join(daemon.project, "src"), { recursive: true });
    writeFileSync(
      join(daemon.project, "src", "lib.rs"),
      "// TODO: fix the bug in the parser\nfn a() {}\n// TODO: tidy the module\nfn b() {}\n",
    );
    await run(daemon, "oxplow.collector.sync", { collector: "triage/todos" });
    await until("the TODOs to be sorted", 30_000, async () => (await facts(daemon)).length === 2);
    expect(await facts(daemon)).toEqual([
      ["src/lib.rs", 1, "bug"],
      ["src/lib.rs", 3, "cleanup"],
    ]);
    await page.goto("/");
    await openFromLauncher(page, "TODO Board");
    const board = page.getByTestId("center-tab-body-oxplow:oxplow/page?page=triage/todo-board");
    const frame = board.frameLocator('[data-testid="custom-component-frame"]');
    const card = (line: number) => frame.locator(".card", { hasText: `src/lib.rs:${line}` });
    await expect(frame.locator(".column", { hasText: "bug" }).locator(".card")).toHaveCount(1);
    await expect(card(3)).toBeVisible();
    await expect(frame.locator("#status")).toHaveText("");

    // A decision from the board's card form.
    await card(1).locator("select[name=decision]").selectOption("fix-now");
    await card(1).getByRole("button", { name: "Save" }).click();
    await until("the board's decision to be stored", 15_000, async () => (await decisions(daemon))["src/lib.rs:1"] === "fix-now");

    // A decision from the `todos` page's `Decide…` row action, in its slideover.
    await openFromLauncher(page, "TODOs");
    const list = page.getByTestId("center-tab-body-oxplow:oxplow/page?page=triage/todos");
    await list.getByRole("row").filter({ hasText: "cleanup" }).locator("td").first().click({ button: "right" });
    await page.getByTestId("menu-item-lens-action-decide").click();
    const form = page.getByTestId("lens-action-form");
    await form.getByTestId("lens-action-form-fields-decision").selectOption("wontfix");
    await form.getByTestId("lens-action-form-fields-submit").click();
    await expect(form).toHaveCount(0);
    await until("the page's decision to be stored", 15_000, async () => (await decisions(daemon))["src/lib.rs:3"] === "wontfix");

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
