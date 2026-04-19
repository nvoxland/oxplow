// Close the dogfood loop started by dogfood-cycle.ts: the agent did the work
// but no commit point existed, so it couldn't propose_commit. Add a commit
// point now, re-prompt the agent, wait for propose_commit, screenshot.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { launchNewde } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchNewde(projectDir);
  const log = (...args: unknown[]) => console.log("[close]", ...args);

  try {
    await window.waitForTimeout(3_000);

    // Activate Work panel.
    const workPanel = window.getByTestId("dock-panel-plan");
    for (let i = 0; i < 3; i++) {
      if ((await workPanel.getAttribute("data-active")) === "true" && (await workPanel.isVisible())) break;
      await window.getByTestId("dock-tab-plan").click();
      await window.waitForTimeout(300);
    }

    // Click "+ Commit when done".
    log("step 1: add commit point");
    const commitBtn = window.getByRole("button", { name: "+ Commit when done" }).first();
    await commitBtn.waitFor({ state: "visible", timeout: 5_000 });
    await commitBtn.click();
    await window.waitForTimeout(800);
    await window.screenshot({ path: resolve(outDir, "close-01-commit-point-added.png") });
    log("  commit point added");

    // Re-prompt the agent in terminal.
    log("step 2: re-prompt agent");
    const xterm = window.locator(".xterm").first();
    await xterm.click();
    await window.waitForTimeout(300);
    const prompt = "A commit point was just added to the queue after the work item. Please continue and call mcp__newde__propose_commit for it with a good conventional-commits message.";
    await window.keyboard.type(prompt);
    await window.keyboard.press("Enter");
    log("  prompt sent");

    // Poll.
    const deadline = Date.now() + 8 * 60_000;
    let tick = 0;
    while (Date.now() < deadline) {
      tick += 1;
      await window.waitForTimeout(10_000);
      const snap = resolve(outDir, `close-02-poll-${String(tick).padStart(2, "0")}.png`);
      await window.screenshot({ path: snap });
      const rows = await window.evaluate(() => {
        const r = document.querySelector(".xterm-rows");
        return r ? (r as HTMLElement).innerText : "";
      });
      writeFileSync(resolve(outDir, `close-02-rows-${String(tick).padStart(2, "0")}.txt`), rows);
      const hints = await window.evaluate(() => {
        const t = document.body.innerText;
        const hits: string[] = [];
        if (/Approve/i.test(t)) hits.push("Approve");
        if (/Reject/i.test(t)) hits.push("Reject");
        if (/awaiting/i.test(t)) hits.push("awaiting");
        if (/pending commit/i.test(t)) hits.push("pending commit");
        return hits;
      });
      log(`  tick=${tick} hints=${JSON.stringify(hints)}`);
      if (hints.includes("Approve") || hints.includes("awaiting")) {
        log("  approval UI visible — stopping poll");
        break;
      }
    }

    await window.screenshot({ path: resolve(outDir, "close-03-final.png") });
    log("DONE — review close-03-final.png; next step is to approve via newde UI.");
  } finally {
    await close();
  }
}

main().catch((err) => { console.error("[close] failed:", err); process.exit(1); });
