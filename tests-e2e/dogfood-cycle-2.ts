// Second dogfood loop: have the inner agent add proper label/htmlFor pairs
// to the new-work-item form, run tests, and commit directly via git
// (the "ad-hoc commit" path — no commit point involved).
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { launchNewde } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const WORK_ITEM_TITLE = "Add <label> htmlFor associations to the new-work-item modal";
const WORK_ITEM_BODY = `The new-work-item modal (opened by the "+ New work item" button in the Work panel) has inputs with placeholder text but no associated <label> elements. Screen readers can't announce what each field is.

In the modal form, every input/select/textarea should have a proper <label> with htmlFor pointing to the field's id. Currently the fields have data-testids but no id/label. Add matching id + <label htmlFor="…"> pairs.

Please:
1. Find the form in src/ui/components/Plan/PlanPane.tsx — it's the NewWorkItemModal component's form, currently containing fields with data-testid like work-item-title, work-item-priority, work-item-description, work-item-acceptance (and optionally work-item-parent).
2. For each field, add an id attribute matching its testid and a <label htmlFor="…"> with text that describes it ("Title", "Priority", "Description", "Acceptance criteria", "Parent epic"). Keep the current visual layout — labels can sit above fields, or you can use visually-hidden labels with sr-only-style CSS if stacking hurts the compact modal. Pick whichever looks cleaner.
3. Don't change other files. Don't change existing testids. Don't alter the modal's onSubmit or validation logic.
4. Run bun test and confirm 292/292 still pass.
5. Commit directly using git: git add -A && git commit -m "<conventional commits message>". DO NOT call propose_commit — there is no commit point, this is the ad-hoc path. Just run git commit yourself.

After the commit, stop.
`;

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchNewde(projectDir);
  const log = (...args: unknown[]) => console.log("[cycle2]", ...args);

  try {
    await window.waitForTimeout(3_000);

    const workPanel = window.getByTestId("dock-panel-plan");
    for (let i = 0; i < 3; i++) {
      if ((await workPanel.getAttribute("data-active")) === "true" && (await workPanel.isVisible())) break;
      await window.getByTestId("dock-tab-plan").click();
      await window.waitForTimeout(300);
    }

    log("step 1: create work item");
    await window.getByTestId("plan-new-work-item").click();
    await window.waitForTimeout(400);
    await window.getByTestId("work-item-title").fill(WORK_ITEM_TITLE);
    await window.getByTestId("work-item-description").fill(WORK_ITEM_BODY);
    await window.getByTestId("work-item-save").click();
    await window.waitForTimeout(800);
    await window.screenshot({ path: resolve(outDir, "c2-01-item-saved.png") });
    log("  work item saved");

    log("step 2: focus terminal, prompt agent");
    const xterm = window.locator(".xterm").first();
    await xterm.click();
    await window.waitForTimeout(300);
    const prompt = `There's a new work item in the queue titled "${WORK_ITEM_TITLE}". Call mcp__newde__list_ready_work to see it, then pick it up, do the work exactly as described in its description, run bun test, and commit with git directly (not propose_commit — this is the ad-hoc commit path).`;
    await window.keyboard.type(prompt);
    await window.keyboard.press("Enter");
    log("  prompt sent");

    // Poll for signs the agent committed.
    const deadline = Date.now() + 10 * 60_000;
    let tick = 0;
    while (Date.now() < deadline) {
      tick += 1;
      await window.waitForTimeout(10_000);
      const snap = resolve(outDir, `c2-02-poll-${String(tick).padStart(2, "0")}.png`);
      await window.screenshot({ path: snap });
      const rows = await window.evaluate(() => {
        const r = document.querySelector(".xterm-rows");
        return r ? (r as HTMLElement).innerText : "";
      });
      writeFileSync(resolve(outDir, `c2-02-rows-${String(tick).padStart(2, "0")}.txt`), rows);
      const hits: string[] = [];
      if (/\[main [0-9a-f]{7,}\]/.test(rows)) hits.push("commit-landed");
      if (/files? changed/i.test(rows)) hits.push("files-changed-line");
      if (/error/i.test(rows)) hits.push("error");
      if (/bun test/.test(rows)) hits.push("bun-test");
      if (/292 pass/.test(rows)) hits.push("292-pass");
      const agentStatus = await window.evaluate(() => {
        const dot = document.querySelector("[data-agent-status]");
        return dot ? dot.getAttribute("data-agent-status") : null;
      });
      log(`  tick=${tick} agentStatus=${agentStatus} hits=${JSON.stringify(hits)}`);
      if (hits.includes("commit-landed") && agentStatus !== "working") {
        log("  commit landed + agent idle — loop complete");
        break;
      }
    }

    await window.screenshot({ path: resolve(outDir, "c2-03-final.png") });
    log("DONE — check git log for the agent's commit");
  } finally {
    await close();
  }
}

main().catch((err) => { console.error("[cycle2] failed:", err); process.exit(1); });
