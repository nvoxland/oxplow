// Ad-hoc dogfood loop: have the inner agent edit code and commit
// directly with `git` (no commit point). Same scaffold as dogfood-cycle,
// just no commit point added and a different prompt.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { dogfoodInnerAgent, launchNewde, probeLog, runProbe, waitForNewdeReady } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const WORK_ITEM_TITLE = "Add <label> htmlFor associations to the new-work-item modal";
const WORK_ITEM_BODY = `The new-work-item modal has inputs without associated <label> elements. Add id attributes matching the existing data-testids and <label htmlFor="…"> wrappers in src/ui/components/Plan/PlanPane.tsx (NewWorkItemModal). Don't touch other files, don't change testids, don't alter validation. Run bun test, then commit directly with git (this is the ad-hoc commit path — do NOT call propose_commit).`;

async function main() {
  await runProbe("dogfood-cycle-2", async () => {
    const projectDir = resolve(__dirname, "..");
    const outDir = resolve(__dirname, "screenshots");
    mkdirSync(outDir, { recursive: true });

    const { window, close } = await launchNewde(projectDir);
    try {
      await waitForNewdeReady(window);
      const result = await dogfoodInnerAgent(window, {
        slug: "cycle2",
        outDir,
        workItemTitle: WORK_ITEM_TITLE,
        workItemBody: WORK_ITEM_BODY,
        prompt: `There's a new work item titled "${WORK_ITEM_TITLE}". Call mcp__newde__list_ready_work to see it, pick it up, do the work, run bun test, and commit directly with git (not propose_commit — this is the ad-hoc path).`,
        addCommitPoint: false,
      });
      probeLog(`[cycle2] done ticks=${result.ticks} exit=${result.exitReason}`);
    } finally {
      await close();
    }
  });
}

main().catch((err) => { console.error("[cycle2] failed:", err); process.exit(1); });
