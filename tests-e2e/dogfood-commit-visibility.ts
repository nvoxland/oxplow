// Dogfood: surface "no active commit point" state in the Work panel.
// Follows the prompt's updated canonical flow: click + Commit when
// done FIRST, then create one work item for visibility, then prompt
// the inner agent. This way propose_commit has something to propose
// against.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { launchNewde, probeLog, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const WORK_ITEM_TITLE = "Surface 'no active commit point' state in Work panel";
const WORK_ITEM_BODY = `When the agent finishes work but no commit point is queued in the batch, propose_commit silently no-ops — the user sees no signal in the Work panel that a commit was ever wanted. This breaks the dogfood loop: outer users don't know whether to click + Commit or wait.

Please:
1. Read .context/agent-model.md to understand the Stop-hook / commit-point flow. Note that buildCommitPointStopReason (src/electron/runtime.ts ~L1684) only fires when a commit point IS active.
2. Find where the Stop-hook pipeline decides "no commit point, just stop" — likely in the same file around the queue-empty path. Add a one-time note or a work-item-level banner shown when: a work item reached 'human_check' or 'done' in a batch with no remaining commit points in that batch's queue.
3. Keep scope TIGHT: a single inline hint in the Work panel when it detects the state "batch has human_check or done items but zero remaining commit_points waiting". Not a modal. Not a toast. An inline note.
4. Update .context/ipc-and-stores.md or .context/agent-model.md with the new signal in the same commit.
5. Run bun test. Do not add new unit tests unless existing ones break.
6. Propose a commit via mcp__newde__propose_commit against the active commit point (there is one — the probe added it).

Keep the scope narrow: one UI note + doc update. No new stores, no new IPC methods, no typing sweeps.`;

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchNewde(projectDir);
  try {
    await window.waitForTimeout(3_000);
    probeLog("[cv] newde launched");

    // Activate Work panel
    const workPanel = window.getByTestId("dock-panel-plan");
    for (let i = 0; i < 3; i++) {
      if ((await workPanel.getAttribute("data-active")) === "true" && (await workPanel.isVisible())) break;
      await window.getByTestId("dock-tab-plan").click();
      await window.waitForTimeout(300);
    }
    probeLog("[cv] Work panel active");

    // Step 1: click + Commit when done FIRST (per updated prompt)
    const addCommitBtn = window.getByTestId("plan-add-commit-point");
    await addCommitBtn.waitFor({ timeout: 5_000 });
    await addCommitBtn.click();
    await window.waitForTimeout(400);
    probeLog("[cv] + Commit when done clicked");
    await window.screenshot({ path: resolve(outDir, "cv-01-commit-point-added.png") });

    // Step 2: create one work item for Plan UI visibility
    await window.getByTestId("plan-new-work-item").click();
    await window.waitForTimeout(400);
    await window.getByTestId("work-item-title").fill(WORK_ITEM_TITLE);
    await window.getByTestId("work-item-description").fill(WORK_ITEM_BODY);
    await window.getByTestId("work-item-save").click();
    await window.waitForTimeout(800);
    probeLog("[cv] work item created");

    // Step 3: prompt inner agent
    const xterm = window.locator(".xterm").first();
    await xterm.waitFor({ state: "visible", timeout: 5_000 });
    await xterm.click();
    await window.waitForTimeout(400);
    const prompt = `There's a new work item titled "${WORK_ITEM_TITLE}". Call mcp__newde__list_ready_work to see it, pick it up, do the work as described, run bun test, and propose the commit when done.`;
    await window.keyboard.type(prompt);
    await window.waitForTimeout(500);
    await window.keyboard.press("Enter");
    probeLog("[cv] prompt sent; polling");

    // Step 4: poll
    const deadlineMs = Date.now() + 10 * 60_000;
    let tick = 0;
    let sawPropose = false;
    let lastRows = "";
    let quiet = 0;
    while (Date.now() < deadlineMs) {
      tick += 1;
      await window.waitForTimeout(15_000);
      await window.screenshot({ path: resolve(outDir, `cv-poll-${String(tick).padStart(2, "0")}.png`) });
      const rows = await window.evaluate(() => {
        const r = document.querySelector(".xterm-rows");
        return r ? (r as HTMLElement).innerText : "";
      });
      writeFileSync(resolve(outDir, `cv-rows-${String(tick).padStart(2, "0")}.txt`), rows);
      const signals = await window.evaluate(() => {
        const body = document.body.innerText;
        return {
          propose: /propose_commit|Propose commit/.test(body),
          pending: /pending commit/i.test(body),
        };
      });
      probeLog(`[cv] tick=${tick} propose=${signals.propose}`);
      if (signals.propose) sawPropose = true;
      if (rows === lastRows) quiet += 1; else quiet = 0;
      lastRows = rows;
      if (sawPropose && quiet >= 2) {
        probeLog("[cv] propose visible and terminal idle; stopping");
        break;
      }
      if (quiet >= 4) {
        probeLog("[cv] terminal quiet for 4 ticks; stopping");
        break;
      }
    }
    probeLog(`[cv] poll done ticks=${tick} sawPropose=${sawPropose}`);
    await window.screenshot({ path: resolve(outDir, "cv-final.png") });
  } finally {
    await close();
  }
}

runProbe("dogfood-commit-visibility", main, { wallMs: 12 * 60_000, silenceMs: 90_000 }).catch(() => process.exit(1));
