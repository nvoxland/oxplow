// Dogfood pass: prompt the inner agent to add an "Include untracked"
// toggle to the Files-commit dialog, defaulting OFF, so probe files
// and stray local-only files stop riding into feature commits. This
// finding has surfaced in 3 consecutive dogfood passes.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchNewde, dogfoodInnerAgent, approveViaFiles, probeLog, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const TITLE = "Files-commit: include-untracked toggle, default OFF";
const BODY = `The Files-commit dialog (CommitDialog in src/ui/components/Panels/ProjectPanel.tsx ~line 1185) calls gitCommitAll, which runs \`git add -A\` — staging untracked files. Three consecutive dogfood passes have shipped commits that bundled local-only files (probe scripts, lock files) by accident.

Please:
1. Add an \`includeUntracked\` boolean to gitCommitAll in src/git/git.ts. When false, use \`git add -u\` (modified + deleted only). When true, keep current \`git add -A\` behavior.
2. Thread the parameter through src/electron/runtime.ts (gitCommitAll), src/electron/main.ts (newde:gitCommitAll handler), and src/ui/api.ts (gitCommitAll wrapper).
3. In CommitDialog, add a checkbox labeled "Include N untracked file(s)" that defaults OFF. The N comes from the indexedFiles list filtered by gitStatus === "untracked"; if N is 0, hide the checkbox entirely. Pass the checkbox state through to gitCommitAll.
4. Update the dialog footer text "Runs \`git add -A &amp;&amp; git commit -m …\`" to reflect the chosen behavior (e.g. "Runs \`git add -u\`" when off).
5. Update .context/data-model.md if it covers commit flow; otherwise mention the change in whichever .context/*.md doc fits.
6. Adjust any existing bun:test that references gitCommitAll's signature. Run bun test. Don't add new tests unless something obviously needs covering.
7. Propose a commit. Conventional-commits style.

Do NOT touch .self-ralph/ or tests-e2e/.`;

const PROMPT = `There is one work item in the queue titled "${TITLE}". Call mcp__newde__list_ready_work to confirm, pick it up, complete it as described, run bun test, and propose a commit. Ignore any other queued items.`;

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  {
    const { window, close } = await launchNewde(projectDir);
    try {
      await window.waitForTimeout(3_000);
      probeLog("[untracked] phase 1 launched");
      const { ticks, exitReason } = await dogfoodInnerAgent(window, {
        slug: "untracked",
        outDir,
        workItemTitle: TITLE,
        workItemBody: BODY,
        prompt: PROMPT,
        tickMs: 15_000,
        quietTicks: 4,
        maxTicks: 40,
      });
      probeLog(`[untracked] inner agent done ticks=${ticks} reason=${exitReason}`);
    } finally {
      await close();
    }
  }

  {
    const { window, close } = await launchNewde(projectDir);
    try {
      await window.waitForTimeout(3_000);
      probeLog("[untracked] phase 2 launched (approve)");
      // NOTE: with the new toggle defaulting OFF, the probe file itself
      // (untracked) should NOT be bundled into the agent's commit. That
      // is the whole point of the fix — verify in step 7.
      await approveViaFiles(window, {
        slug: "untracked",
        outDir,
        message: "feat(files): include-untracked toggle in commit dialog, default OFF",
      });
      probeLog("[untracked] approval done");
    } finally {
      await close();
    }
  }
}

runProbe("dogfood-untracked-toggle", main, { wallMs: 14 * 60_000, silenceMs: 100_000 }).catch(() => process.exit(1));
