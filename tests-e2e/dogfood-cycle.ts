// Canonical dogfood loop driven through newde's UI:
//   1. Add a commit point (so propose_commit has a target).
//   2. Create one work item describing the desired edit.
//   3. Type a prompt into the agent xterm.
//   4. Poll until the agent goes quiet.
// Approval is left to the user (or a follow-up cycle via dogfood-cycle-close.ts).
//
// Uses dogfoodInnerAgent from harness.ts. Earlier versions of this file,
// dogfood-cycle-2.ts, and dogfood-cycle-close.ts each open-coded the
// same loop; they now share this helper.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { dogfoodInnerAgent, launchNewde, probeLog, runProbe, waitForNewdeReady } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const WORK_ITEM_TITLE = "Add aria-labels to emoji sort buttons in PlanPane";
const WORK_ITEM_BODY = `The four small buttons next to the thread status bar in PlanPane show glyphs ⤡ ⤢ ↓ ↑ with no aria-label and no title attribute. Hovering reveals nothing; a screen reader gets an empty button.

Please:
1. Find those buttons in src/ui/components/Plan/PlanPane.tsx (they sit in the status bar near the "Backlog · N" button).
2. Add a descriptive aria-label AND a matching title attribute to each button based on what it actually does.
3. Do not change any other files. Do not change styling.
4. After the edit, run bun test.
5. Propose a commit with a conventional-commits-style message.
`;

async function main() {
  await runProbe("dogfood-cycle", async () => {
    const projectDir = resolve(__dirname, "..");
    const outDir = resolve(__dirname, "screenshots");
    mkdirSync(outDir, { recursive: true });

    const { window, close } = await launchNewde(projectDir);
    try {
      await waitForNewdeReady(window);
      const result = await dogfoodInnerAgent(window, {
        slug: "cycle",
        outDir,
        workItemTitle: WORK_ITEM_TITLE,
        workItemBody: WORK_ITEM_BODY,
        prompt: `There's a new work item in the queue titled "${WORK_ITEM_TITLE}". Call mcp__newde__list_ready_work to see it, then pick it up (mark it in_progress), do the work as described, run bun test, and propose a commit when done.`,
        addCommitPoint: true,
      });
      probeLog(`[cycle] done ticks=${result.ticks} exit=${result.exitReason}`);
    } finally {
      await close();
    }
  });
}

main().catch((err) => { console.error("[cycle] failed:", err); process.exit(1); });
