// Dogfood pass: prompt the inner agent to make the Files-pane filter
// affordance discoverable. The trigger is currently an unlabeled eye
// icon (no aria-label, no testid), so users scanning for "Filter"
// text find nothing — the pass-1 verify probe in
// fix-20260419-223716-blame-ctxmenu.md returned [] when scanning
// buttons for /uncommitted|branch|upstream/.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchNewde, dogfoodInnerAgent, approveViaFiles, probeLog, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const TITLE = "Make Files-pane filter affordance discoverable";
const BODY = `In src/ui/components/Panels/ProjectPanel.tsx (around line 1513) the file-tree filter is an eye icon button with NO aria-label and NO data-testid. The popover menu items (All files, Uncommitted changes, Branch changes, Unpushed changes, Turn) also have no testids. As a result:
- Screen-reader users hear "button" with nothing else.
- Probes that scan for "Filter" / "uncommitted" / "branch" buttons find zero matches.
- New users hovering see a tooltip but the button is invisible until hovered.

Please:
1. Add aria-label="Filter files" to the eye-icon trigger button.
2. Add data-testid="files-filter-toggle" to the same button.
3. Add data-testid={\`files-filter-option-\${opt.value}\`} to each option button in the popover (values: all, uncommitted, branch, unpushed, turn).
4. Update .context/data-model.md to document the FilterMode values and the popover trigger so future docs match the UI.
5. Run \`bun test\`. Add no new tests.
6. Propose a commit. Conventional-commits style.

Do NOT touch .self-ralph/ or tests-e2e/.`;

const PROMPT = `There is one work item in the queue titled "${TITLE}". Call mcp__newde__list_ready_work to confirm, pick it up, complete it as described in the body, run bun test, and propose a commit when done. Ignore any other queued items.`;

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  {
    const { window, close } = await launchNewde(projectDir);
    try {
      await window.waitForTimeout(3_000);
      probeLog("[files-filter] phase 1 launched");
      const { ticks, exitReason } = await dogfoodInnerAgent(window, {
        slug: "files-filter",
        outDir,
        workItemTitle: TITLE,
        workItemBody: BODY,
        prompt: PROMPT,
        tickMs: 15_000,
        quietTicks: 4,
        maxTicks: 32,
      });
      probeLog(`[files-filter] inner agent done ticks=${ticks} reason=${exitReason}`);
    } finally {
      await close();
    }
  }

  {
    const { window, close } = await launchNewde(projectDir);
    try {
      await window.waitForTimeout(3_000);
      probeLog("[files-filter] phase 2 launched");
      await approveViaFiles(window, {
        slug: "files-filter",
        outDir,
        message: "feat(files): label and testid the file-filter affordance for discoverability",
      });
      probeLog("[files-filter] approval done");
    } finally {
      await close();
    }
  }
}

runProbe("dogfood-files-filter", main, { wallMs: 12 * 60_000, silenceMs: 90_000 }).catch(() => process.exit(1));
