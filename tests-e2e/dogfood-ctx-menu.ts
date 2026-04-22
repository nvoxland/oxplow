// Dogfood pass: prompt the inner agent (inside newde) to expand the
// work-item right-click context menu beyond Delete. Adapted from
// dogfood-cycle.ts but narrower in scope.
//
// This script drives newde as a user:
//   1. Create ONE work item via + New (the allowed Plan-UI exception)
//   2. Focus the agent-pane xterm
//   3. Type a prompt referencing the work item by title
//   4. Poll and screenshot until the agent proposes a commit or we
//      hit the watchdog budget
//   5. If an approve-commit affordance appears, click it via the UI.
//
// Emits heartbeats every poll so runProbe's silence watchdog stays
// happy during long thinking pauses from the inner agent.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { launchNewde, probeLog, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const WORK_ITEM_TITLE = "Expand work-item right-click menu beyond Delete";
const WORK_ITEM_BODY = `The work-item row right-click context menu currently only has one entry: Delete. Rename and status/priority changes require hover or double-click, which is keyboard-hostile and invisible to new users.

Please:
1. Find the work-item context menu definition (likely in src/ui/components/Plan/ — search for "workitem.delete" or similar MenuItem id).
2. Add a "Rename…" menu item that triggers the existing inline rename flow on the row (same effect as double-click).
3. If the existing context menu plumbing makes it trivial, also add "Change status…" and "Change priority…" entries that open the existing s/p keyboard pickers. If not trivial, skip these — one entry is fine for this pass.
4. Run bun test. Do not add new tests unless existing ones break.
5. Propose a commit with a conventional-commits-style message that references the pattern established in commit 7cc3302 (ThreadRail context-menu expansion).

Do not touch .self-ralph/ or tests-e2e/.`;

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchNewde(projectDir);

  try {
    await window.waitForTimeout(3_000);
    probeLog("[dogfood] newde launched");

    // Step 1: activate Work panel
    const workPanel = window.getByTestId("dock-panel-plan");
    for (let i = 0; i < 3; i++) {
      if ((await workPanel.getAttribute("data-active")) === "true" && (await workPanel.isVisible())) break;
      await window.getByTestId("dock-tab-plan").click();
      await window.waitForTimeout(300);
    }
    probeLog("[dogfood] Work panel active");

    // Step 2: create ONE work item (allowed single-item exception)
    await window.getByTestId("plan-new-task").click();
    await window.waitForTimeout(400);
    await window.getByTestId("work-item-title").fill(WORK_ITEM_TITLE);
    await window.getByTestId("work-item-description").fill(WORK_ITEM_BODY);
    await window.screenshot({ path: resolve(outDir, "dogfood-01-form.png") });
    await window.getByTestId("work-item-save").click();
    await window.waitForTimeout(800);
    probeLog("[dogfood] work item created");

    // Step 3: focus xterm and prompt
    const xterm = window.locator(".xterm").first();
    await xterm.waitFor({ state: "visible", timeout: 5_000 });
    await xterm.click();
    await window.waitForTimeout(400);

    const prompt = `There's a new work item in the queue titled "${WORK_ITEM_TITLE}". Call mcp__newde__list_ready_work to see it, pick it up (mark in_progress), do the work as described, run bun test, and propose a commit when done.`;
    await window.keyboard.type(prompt);
    await window.waitForTimeout(500);
    await window.screenshot({ path: resolve(outDir, "dogfood-02-prompt.png") });
    await window.keyboard.press("Enter");
    probeLog("[dogfood] prompt sent; starting poll");

    // Step 4: poll loop with heartbeats
    const deadlineMs = Date.now() + 8 * 60_000;
    let tick = 0;
    let sawPropose = false;
    let lastRowsSnapshot = "";
    while (Date.now() < deadlineMs) {
      tick += 1;
      await window.waitForTimeout(15_000);
      const snapPath = resolve(outDir, `dogfood-poll-${String(tick).padStart(2, "0")}.png`);
      await window.screenshot({ path: snapPath });
      const rowsText = await window.evaluate(() => {
        const rows = document.querySelector(".xterm-rows");
        return rows ? (rows as HTMLElement).innerText : "";
      });
      writeFileSync(resolve(outDir, `dogfood-rows-${String(tick).padStart(2, "0")}.txt`), rowsText);

      const signals = await window.evaluate(() => {
        const bodyText = document.body.innerText;
        const buttons = Array.from(document.querySelectorAll("button"));
        const approveBtn = buttons.find((b) => /approve/i.test(b.textContent ?? "") || /approve/i.test(b.getAttribute("title") ?? ""))
          ?? document.querySelector('[data-testid^="approve"]');
        return {
          proposeCommit: /propose_commit|Propose commit/.test(bodyText),
          pendingCommit: /pending commit/i.test(bodyText),
          approveBtn: !!approveBtn,
          agentStatus: document.querySelector("[data-agent-status]")?.getAttribute("data-agent-status") ?? null,
        };
      });
      probeLog(`[dogfood] tick=${tick} status=${signals.agentStatus} propose=${signals.proposeCommit} approveBtn=${signals.approveBtn}`);

      if (!sawPropose && signals.proposeCommit) {
        sawPropose = true;
        probeLog("[dogfood] propose_commit visible");
      }
      if (sawPropose && signals.agentStatus !== "working") {
        probeLog("[dogfood] agent idle with propose visible; stopping poll");
        break;
      }
      // Cheap liveness: if terminal content hasn't changed for 3 ticks
      // AND no propose, break early to conserve budget.
      if (rowsText === lastRowsSnapshot && tick > 3 && !sawPropose) {
        probeLog("[dogfood] terminal quiet for 3 ticks; stopping early");
        break;
      }
      lastRowsSnapshot = rowsText;
    }

    await window.screenshot({ path: resolve(outDir, "dogfood-final.png") });
    probeLog(`[dogfood] poll ended: sawPropose=${sawPropose}, ticks=${tick}`);
    probeLog("[dogfood] NOT clicking approve — user will review manually");
  } finally {
    await close();
  }
}

runProbe("dogfood-ctx-menu", main, { wallMs: 10 * 60_000, silenceMs: 90_000 }).catch(() => process.exit(1));
