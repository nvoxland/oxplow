// Dogfood pass 3/3 (rotation branch b): have the inner agent add a
// data-agent-status attribute that probes can read without focusing
// the agent tab. Follows the canonical flow: + Commit → + New →
// prompt xterm → poll → approve.
//
// Pass 2 surfaced that stale work items in the repo's
// .oxplow/state.sqlite get picked up by Stop-hook and pull the agent
// sideways. Mitigation in the prompt: name "ignore any other queued
// items — only do the one titled exactly X" explicitly.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { launchOxplow, probeLog, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const WORK_ITEM_TITLE = "Expose data-agent-status on AgentStatusDot";
const WORK_ITEM_BODY = `Probes can't poll agent state from outside the active tab — document.querySelector("[data-agent-status]") returns nothing. Surfaced by fix-20260419-220229-ctxmenu-dogfood.md.

Scope (TIGHT):
1. In src/ui/components/AgentStatusDot.tsx, add data-agent-status={status} and data-agent-label={LABELS[status]} attributes to the <span>. That's it. One file, two new attributes.
2. Verify bun test still passes (296 tests).
3. Propose a commit against the active commit point (the probe added one).

Do NOT touch other files. Do NOT add tests. Do NOT pick up any other work items in the queue even if Stop-hook suggests them — ignore them. Focus exclusively on the work item titled exactly "${WORK_ITEM_TITLE}".`;

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchOxplow(projectDir);
  try {
    await window.waitForTimeout(3_000);
    probeLog("[as] oxplow launched");

    await window.getByTestId("rail-page-plan-work").click();
    await window.getByTestId("page-plan-work").waitFor({ state: "visible", timeout: 5_000 });

    await window.getByTestId("plan-add-commit-point").click();
    await window.waitForTimeout(400);
    probeLog("[as] + Commit when done");

    await window.getByTestId("plan-new-task").click();
    await window.waitForTimeout(400);
    await window.getByTestId("work-item-title").fill(WORK_ITEM_TITLE);
    await window.getByTestId("work-item-description").fill(WORK_ITEM_BODY);
    await window.getByTestId("work-item-save").click();
    await window.waitForTimeout(800);
    probeLog("[as] work item created");

    const xterm = window.locator(".xterm").first();
    await xterm.waitFor({ state: "visible", timeout: 5_000 });
    await xterm.click();
    await window.waitForTimeout(400);
    const prompt = `There's a new work item titled "${WORK_ITEM_TITLE}". Pick up only this one — ignore any other queued items. Do the scope described and propose the commit.`;
    await window.keyboard.type(prompt);
    await window.waitForTimeout(500);
    await window.keyboard.press("Enter");
    probeLog("[as] prompt sent");

    const deadlineMs = Date.now() + 8 * 60_000;
    let tick = 0;
    let lastRows = "";
    let quiet = 0;
    // Don't trust the scrollback "propose" match — pass 2 showed
    // it's polluted by prior sessions. Gate on quiet-for-3-ticks
    // instead of propose signal.
    while (Date.now() < deadlineMs) {
      tick += 1;
      await window.waitForTimeout(15_000);
      await window.screenshot({ path: resolve(outDir, `as-poll-${String(tick).padStart(2, "0")}.png`) });
      const rows = await window.evaluate(() => {
        const r = document.querySelector(".xterm-rows");
        return r ? (r as HTMLElement).innerText : "";
      });
      writeFileSync(resolve(outDir, `as-rows-${String(tick).padStart(2, "0")}.txt`), rows);
      if (rows === lastRows) quiet += 1; else quiet = 0;
      lastRows = rows;
      probeLog(`[as] tick=${tick} quiet=${quiet}`);
      if (quiet >= 3) {
        probeLog("[as] terminal quiet for 3 ticks; stopping");
        break;
      }
    }
    await window.screenshot({ path: resolve(outDir, "as-final.png") });
    probeLog(`[as] done after ${tick} ticks`);
  } finally {
    await close();
  }
}

runProbe("dogfood-agent-status-attr", main, { wallMs: 10 * 60_000, silenceMs: 90_000 }).catch(() => process.exit(1));
