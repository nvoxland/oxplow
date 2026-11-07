import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";

import { run } from "../../support/daemon.js";
import { expect, test } from "../../support/fixtures.js";
import { open } from "../../support/ui.js";

// The Assistant: a page's ☰ → Ask the Assistant opens its dock panel about
// that page; a question typed there is answered by the model bound to the
// `assistant` role (an OpenAI-compatible stand-in here), streamed into the
// transcript.

const chunk = (delta: object, finish: string | null) =>
  `data: ${JSON.stringify({ id: "c", object: "chat.completion.chunk", created: 1, model: "m", choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`;

/** Answers every chat completion with `answer`, as a server-sent stream. */
async function model(answer: string): Promise<{ url: string; close(): Promise<void> }> {
  const server: Server = createServer((req, res) => {
    req.on("data", () => {});
    req.on("end", () => {
      res.writeHead(200, { "content-type": "text/event-stream" });
      res.write(chunk({ role: "assistant", content: answer }, "stop"));
      res.write(
        `data: ${JSON.stringify({ id: "c", object: "chat.completion.chunk", created: 1, model: "m", choices: [], usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 } })}\n\n`,
      );
      res.end("data: [DONE]\n\n");
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = (server.address() as AddressInfo).port;
  return { url: `http://127.0.0.1:${port}`, close: () => new Promise((r) => server.close(() => r())) };
}

test("ask the Assistant about a page and an answer appears in the panel", async ({ page, daemon }) => {
  const stub = await model("Suite Home is a page of the test extension.");
  try {
    await run(daemon, "oxplow.ai.set_provider", { id: "stand-in", kind: "openai_compatible", base_url: stub.url, key: "sk-e2e" });
    await run(daemon, "oxplow.ai.set_role", { role: "assistant", provider: "stand-in", model: "m" });
    await open(page, daemon, "oxplow:oxplow/page?page=e2e/home");
    await page.locator('[data-testid="page-nav-menu"]:visible').click();
    await page.getByTestId("menu-item-page-ask-assistant").click();

    await expect(page.getByTestId("assistant-panel")).toBeVisible();
    await expect(page.getByTestId("assistant-about")).toContainText("Suite Home");
    await page.getByTestId("assistant-input").fill("What is this page for?");
    await page.getByTestId("assistant-input").press("Enter");
    await expect(page.getByTestId("assistant-transcript")).toContainText("What is this page for?");
    await expect(page.getByTestId("assistant-transcript")).toContainText("Suite Home is a page of the test extension.");
    await expect(page.getByTestId("assistant-panel")).toHaveAttribute("data-mode", "idle");
  } finally {
    await stub.close();
  }
});
