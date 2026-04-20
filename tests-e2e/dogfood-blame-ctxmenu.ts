// Dogfood pass: prompt the inner agent to add a right-click context
// menu to the BlameOverlay rows in EditorPane.tsx. Rows currently
// only respond to onClick (reveal commit) — there's no way to copy
// the SHA, open the commit elsewhere, or see the full summary
// without hovering.
//
// Uses the new harness helpers (dogfoodInnerAgent + approveViaFiles)
// extracted in 133cbf0 — first probe to do so. Approval happens in a
// second launch because the first launch's window can be left in a
// less-clean state by the inner agent.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchNewde, dogfoodInnerAgent, approveViaFiles, probeLog, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const TITLE = "Add right-click context menu to blame gutter rows";
const BODY = `The blame gutter (BlameOverlay in src/ui/components/EditorPane.tsx) currently only handles left-click (reveals the commit). There is no right-click context menu. As a user looking at blame, I expect to be able to:
- Copy commit SHA
- Reveal commit (same as left-click; surfaced for discoverability)
- Copy author email

Please:
1. Read src/ui/components/EditorPane.tsx — find the BlameOverlay component (~line 677) and the existing ContextMenu usage in EditorPane (around line 570).
2. Wire an onContextMenu handler on each blame row that opens the existing ContextMenu component with the three items above. Skip the menu entirely for the uncommitted row (sha is all zeros).
3. Reuse the existing ContextMenu component — do not introduce a new one.
4. Update .context/editor-and-monaco.md to mention the blame-row context menu in the same commit.
5. Run \`bun test\` to make sure nothing breaks. Do NOT add new tests unless something obviously needs covering.
6. Propose a commit when done. Conventional-commits style.

Do NOT touch .self-ralph/ or tests-e2e/.`;

const PROMPT = `There is one work item in the queue titled "${TITLE}". Please call mcp__newde__list_ready_work to confirm, then pick it up and complete it as described in its body. Run bun test before proposing the commit. Ignore any other queued items — focus only on this one.`;

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  // Phase 1: launch, prompt inner agent, watch
  {
    const { window, close } = await launchNewde(projectDir);
    try {
      await window.waitForTimeout(3_000);
      probeLog("[blame-ctx] newde launched (phase 1: prompt)");

      const { ticks, exitReason } = await dogfoodInnerAgent(window, {
        slug: "blame-ctx",
        outDir,
        workItemTitle: TITLE,
        workItemBody: BODY,
        prompt: PROMPT,
        tickMs: 15_000,
        quietTicks: 4,
        maxTicks: 36, // ~9 minutes
      });
      probeLog(`[blame-ctx] inner-agent run ended ticks=${ticks} reason=${exitReason}`);
      await window.screenshot({ path: resolve(outDir, "blame-ctx-after-agent.png") });
    } finally {
      await close();
    }
  }

  // Phase 2: relaunch, approve via Files-commit dialog
  {
    const { window, close } = await launchNewde(projectDir);
    try {
      await window.waitForTimeout(3_000);
      probeLog("[blame-ctx] newde launched (phase 2: approve)");
      await approveViaFiles(window, {
        slug: "blame-ctx",
        outDir,
        message: "feat(editor): add right-click context menu to blame gutter rows",
      });
      probeLog("[blame-ctx] approval complete");
    } finally {
      await close();
    }
  }
}

runProbe("dogfood-blame-ctxmenu", main, { wallMs: 14 * 60_000, silenceMs: 90_000 }).catch(() => process.exit(1));
