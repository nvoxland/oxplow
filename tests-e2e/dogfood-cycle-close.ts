// Follow-up dogfood loop: add a commit point and re-prompt the agent
// to propose_commit. Use after a dogfood-cycle pass that left edits
// uncommitted (e.g. early cycle.ts variants that didn't seed a commit
// point). Now that dogfood-cycle.ts seeds a commit point itself this
// is mostly a fallback for partially-completed runs.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { dogfoodInnerAgent, launchNewde, probeLog, runProbe, waitForNewdeReady } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const WORK_ITEM_TITLE = "Propose commit for in-flight changes";
const WORK_ITEM_BODY = "A commit point was just seeded. Call mcp__newde__propose_commit for the pending edits with a conventional-commits message.";

async function main() {
  await runProbe("dogfood-cycle-close", async () => {
    const projectDir = resolve(__dirname, "..");
    const outDir = resolve(__dirname, "screenshots");
    mkdirSync(outDir, { recursive: true });

    const { window, close } = await launchNewde(projectDir);
    try {
      await waitForNewdeReady(window);
      const result = await dogfoodInnerAgent(window, {
        slug: "close",
        outDir,
        workItemTitle: WORK_ITEM_TITLE,
        workItemBody: WORK_ITEM_BODY,
        prompt: WORK_ITEM_BODY,
        addCommitPoint: true,
      });
      probeLog(`[close] done ticks=${result.ticks} exit=${result.exitReason}`);
    } finally {
      await close();
    }
  });
}

main().catch((err) => { console.error("[close] failed:", err); process.exit(1); });
