// Dogfood pass (rotation b — IPC noise): prompt the inner agent to
// suppress the "PostToolUse:Edit hook error / Failed with non-
// blocking status code" wall in xterm. The hook-events panel
// already has the data; the xterm error spam is duplicate noise
// that hides the agent's actual work.
//
// Hypothesis: the response shape from src/mcp/mcp-server.ts ~line
// 247 (status 202, empty body for void onHook returns) is what
// Claude Code is interpreting as a non-blocking failure.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchNewde, dogfoodInnerAgent, approveViaFiles, runBuild, probeLog, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const TITLE = "Suppress PostToolUse:Edit / PreToolUse:Edit xterm error noise";
const BODY = `When the inner agent runs Edit/Write tools, the xterm fills with:

  ⎿  PostToolUse:Edit hook error
     Failed with non-blocking status code: -----...

The hook-events panel (BottomPanel.tsx) cleanly renders the same events without that noise. The fix should make Claude Code stop printing the failure line.

Likely cause: src/mcp/mcp-server.ts ~line 247 returns \`res.statusCode = response?.status ?? 202; res.end();\` when the runtime's onHook returns void. Claude Code may be interpreting "202 with empty body" as a non-blocking failure to ack.

Please:
1. Read src/mcp/mcp-server.ts ~line 230-250 (the hook response logic) and src/electron/runtime.ts ~line 1036 (handleHookEnvelope) to confirm the path that emits 202+empty.
2. Try changing the void-return path to send 200 + JSON \`{}\` (empty object). Claude Code's HTTP-hook spec accepts an empty JSON object as "no directive, success". Verify by checking src/mcp/mcp-server.test.ts that this doesn't break existing assertions.
3. If the test suite has a test exercising the 202 path explicitly, update it to assert 200+{} instead.
4. Update .context/agent-model.md to mention the response shape if it covers hook plumbing.
5. Run \`bun test\`.
6. Propose a commit. Conventional-commits style.

Do NOT touch .self-ralph/ or tests-e2e/.`;

const PROMPT = `There is one work item in the queue titled "${TITLE}". Call mcp__newde__list_ready_work to confirm, pick it up, complete it, run bun test, and propose a commit. Ignore any other queued items.`;

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  {
    const { window, close } = await launchNewde(projectDir);
    try {
      await window.waitForTimeout(3_000);
      probeLog("[hook-noise] phase 1 launched");
      const { ticks, exitReason } = await dogfoodInnerAgent(window, {
        slug: "hook-noise",
        outDir,
        workItemTitle: TITLE,
        workItemBody: BODY,
        prompt: PROMPT,
        tickMs: 15_000,
        quietTicks: 4,
        maxTicks: 36,
      });
      probeLog(`[hook-noise] inner agent done ticks=${ticks} reason=${exitReason}`);
    } finally {
      await close();
    }
  }

  runBuild();

  {
    const { window, close } = await launchNewde(projectDir);
    try {
      await window.waitForTimeout(3_000);
      probeLog("[hook-noise] phase 2 launched (approve)");
      await approveViaFiles(window, {
        slug: "hook-noise",
        outDir,
        message: "fix(mcp): respond 200+{} for void hook returns to silence CC error wall",
      });
      probeLog("[hook-noise] approval done");
    } finally {
      await close();
    }
  }
}

runProbe("dogfood-suppress-hook-noise", main, { wallMs: 14 * 60_000, silenceMs: 100_000 }).catch(() => process.exit(1));
