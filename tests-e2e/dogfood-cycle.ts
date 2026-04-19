// One real dogfood cycle driven entirely through newde's UI via Playwright:
//
//  1. Create a work item describing the improvement (via + New work item)
//  2. Focus the center Agent tab's terminal, type a prompt telling the inner
//     Claude to pick up the queue
//  3. Watch the agent work (poll agent status / terminal content)
//  4. When a commit-approval UI appears, screenshot it for review
//  5. (Approval happens interactively — the harness stops at review.)
//
// The assumption is that the inner agent does the actual code edit; this
// script must never call fs.writeFile on src/**.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { launchNewde } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const WORK_ITEM_TITLE = "Add aria-labels to emoji sort buttons in PlanPane";
const WORK_ITEM_BODY = `The four small buttons next to the batch status bar in PlanPane show glyphs ⤡ ⤢ ↓ ↑ with no aria-label and no title attribute. Hovering reveals nothing; a screen reader gets an empty button.

Please:
1. Find those buttons in src/ui/components/Plan/PlanPane.tsx (they sit in the status bar near the "Backlog · N" button).
2. Add a descriptive aria-label AND a matching title attribute to each button based on what it actually does (look at its onClick to infer meaning — collapse/expand, move up/down, etc.).
3. Do not change any other files. Do not change styling. Only aria-label and title.
4. After the edit, run bun test to confirm 292 still pass.
5. Propose a commit with a conventional-commits-style message.
`;

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchNewde(projectDir);
  const log = (...args: unknown[]) => console.log("[cycle]", ...args);

  try {
    await window.waitForTimeout(3_000);

    // Step 1: Ensure the Work dock tab is active.
    log("step 1: activate Work tab");
    const workPanel = window.getByTestId("dock-panel-plan");
    for (let i = 0; i < 3; i++) {
      if ((await workPanel.getAttribute("data-active")) === "true" && (await workPanel.isVisible())) break;
      await window.getByTestId("dock-tab-plan").click();
      await window.waitForTimeout(300);
    }

    // Step 2: Create a work item.
    log("step 2: open + New work item");
    await window.getByTestId("plan-new-work-item").click();
    await window.waitForTimeout(400);
    await window.getByTestId("work-item-title").fill(WORK_ITEM_TITLE);
    await window.getByTestId("work-item-description").fill(WORK_ITEM_BODY);
    await window.screenshot({ path: resolve(outDir, "cycle-01-form.png") });
    await window.getByTestId("work-item-save").click();
    await window.waitForTimeout(800);
    await window.screenshot({ path: resolve(outDir, "cycle-02-item-created.png") });
    log("  work item saved");

    // Step 3: Click the center Agent tab (it's the first CenterTabs tab,
    // labeled by the selectedBatch title). Instead of relying on a testid
    // we don't have there yet, find the .xterm host and click it.
    log("step 3: focus terminal and send prompt");
    const xterm = window.locator(".xterm").first();
    await xterm.waitFor({ state: "visible", timeout: 5_000 });
    await xterm.click();
    await window.waitForTimeout(400);

    const prompt = `There's a new work item in the queue titled "${WORK_ITEM_TITLE}". Call mcp__newde__list_ready_work to see it, then pick it up (mark it in_progress), do the work as described, run bun test, and propose a commit when done.`;
    // Type the prompt. xterm-js sees the keystrokes via
    // attachCustomKeyEventHandler → PTY. We send it via page.keyboard.type
    // which gives real keydown events.
    await window.keyboard.type(prompt);
    await window.waitForTimeout(500);
    await window.screenshot({ path: resolve(outDir, "cycle-03-prompt-typed.png") });
    await window.keyboard.press("Enter");
    log("  prompt sent; watching for agent progress");

    // Step 4: Poll — wait for commit-approval UI to appear OR for a timeout.
    // Commit points surface as "Propose commit" / "Approve" affordances in
    // the work panel. We watch the terminal rows for a propose_commit hint,
    // and snapshot every ~10s.
    const deadline = Date.now() + 10 * 60_000;
    let tick = 0;
    let sawPropose = false;
    while (Date.now() < deadline) {
      tick += 1;
      await window.waitForTimeout(10_000);
      const snap = resolve(outDir, `cycle-04-poll-${String(tick).padStart(2, "0")}.png`);
      await window.screenshot({ path: snap });
      const rowsText = await window.evaluate(() => {
        const rows = document.querySelector(".xterm-rows");
        return rows ? (rows as HTMLElement).innerText : "";
      });
      writeFileSync(resolve(outDir, `cycle-04-rows-${String(tick).padStart(2, "0")}.txt`), rowsText);
      const approvalHints = await window.evaluate(() => {
        const bodyText = document.body.innerText;
        const hits: string[] = [];
        if (/propose_commit/i.test(bodyText)) hits.push("propose_commit");
        if (/pending commit/i.test(bodyText)) hits.push("pending commit");
        if (/approve/i.test(bodyText)) hits.push("approve");
        if (/in progress/i.test(bodyText) || /in_progress/i.test(bodyText)) hits.push("in_progress");
        return hits;
      });
      const agentStatus = await window.evaluate(() => {
        const dot = document.querySelector("[data-agent-status]");
        return dot ? dot.getAttribute("data-agent-status") : null;
      });
      log(`  tick=${tick} agentStatus=${agentStatus} hints=${JSON.stringify(approvalHints)}`);
      if (!sawPropose && approvalHints.includes("propose_commit")) {
        sawPropose = true;
        log("  propose_commit visible — agent has reached the commit point");
      }
      // Stop polling if we see the approval UI AND agent has gone idle.
      if (sawPropose && agentStatus !== "working") {
        log("  agent idle with propose_commit visible — stopping poll for user review");
        break;
      }
    }

    // Final screenshot for review.
    await window.screenshot({ path: resolve(outDir, "cycle-05-final.png") });
    log("cycle complete — review cycle-05-final.png and the final rows dump");
    log("IMPORTANT: the harness intentionally does NOT approve the commit; review in newde and approve manually.");
    // Keep the window alive for a bit so the user can interact if launched manually.
  } finally {
    await close();
  }
}

main().catch((err) => {
  console.error("[cycle] failed:", err);
  process.exit(1);
});
