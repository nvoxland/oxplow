// Dogfood pass: prompt the inner agent to make the active Files-pane
// filter VISUALLY discoverable. Today: when filter !== "all" the
// trigger button only changes background color; the user can't tell
// at a glance which filter is active without hovering or opening the
// popover. The fix from `2e097c7` covered screen-reader + probe
// discoverability via aria-label; this one covers human visual
// discoverability via a text chip.
//
// Uses runBuild() between phases so the inner agent's edits take
// effect at approval time (regression-prevention from
// `fix-20260419-225432-untracked-toggle.md`).
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchOxplow, dogfoodInnerAgent, approveViaFiles, runBuild, probeLog, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const TITLE = "Files-pane filter: show active filter as a label, not just color";
const BODY = `In src/ui/components/Panels/ProjectPanel.tsx the FilterMenuButton (~line 1480) renders an eye icon. When a non-"all" filter is active, only its BACKGROUND COLOR changes — the user cannot tell at a glance which filter is active without hovering for the title tooltip or opening the popover.

Please:
1. When filterMode !== "all", render the trigger as an icon-plus-text "chip" showing a short version of the active filter name. Suggested short forms: "Uncommitted" / "Branch" / "Unpushed" / "Turn". When filterMode === "all", keep the icon-only look (don't take up space in the header for the default state).
2. Reuse the existing aria-label, title, and data-testid="files-filter-toggle". Do not change the popover.
3. Update .context/ docs only if the change touches a documented invariant.
4. Run \`bun test\`. Do not add new tests.
5. Propose a commit when done. Conventional-commits style.

Do NOT touch .self-ralph/ or tests-e2e/.`;

const PROMPT = `There is one work item in the queue titled "${TITLE}". Call mcp__oxplow__list_ready_work to confirm, pick it up, complete it, run bun test, and propose a commit. Ignore any other queued items.`;

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  {
    const { window, close } = await launchOxplow(projectDir);
    try {
      await window.waitForTimeout(3_000);
      probeLog("[filter-chip] phase 1 launched");
      const { ticks, exitReason } = await dogfoodInnerAgent(window, {
        slug: "filter-chip",
        outDir,
        workItemTitle: TITLE,
        workItemBody: BODY,
        prompt: PROMPT,
        tickMs: 15_000,
        quietTicks: 4,
        maxTicks: 30,
      });
      probeLog(`[filter-chip] inner agent done ticks=${ticks} reason=${exitReason}`);
    } finally {
      await close();
    }
  }

  // Pick up inner agent's edits before phase 2 — fix-20260419-225432.
  runBuild();

  {
    const { window, close } = await launchOxplow(projectDir);
    try {
      await window.waitForTimeout(3_000);
      probeLog("[filter-chip] phase 2 launched (approve)");
      await approveViaFiles(window, {
        slug: "filter-chip",
        outDir,
        message: "feat(files): show active filter as a chip label, not just color",
      });
      probeLog("[filter-chip] approval done");
    } finally {
      await close();
    }
  }
}

runProbe("dogfood-filter-chip", main, { wallMs: 12 * 60_000, silenceMs: 100_000 }).catch(() => process.exit(1));
