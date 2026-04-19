// Probe newde's agent-dispatch UI: what does the center Agent tab look like?
// Is claude auto-started or do I need to launch it? What does the commit-
// approval flow look like?
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { launchNewde } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchNewde(projectDir);
  const log = (...args: unknown[]) => console.log("[dogfood]", ...args);

  try {
    await window.waitForTimeout(3_000);
    await window.screenshot({ path: resolve(outDir, "df-00-launch.png") });

    // Dump the center area to see the Agent tab state and any pre-existing content.
    const centerInfo = await window.evaluate(() => {
      // Find .xterm canvas / rows / the terminal host.
      const xterm = document.querySelector(".xterm");
      const rows = document.querySelector(".xterm-rows");
      return {
        xtermFound: !!xterm,
        xtermHtml: xterm ? (xterm as HTMLElement).outerHTML.slice(0, 300) : null,
        rowsText: rows ? (rows as HTMLElement).innerText.slice(0, 2000) : null,
      };
    });
    log("center info:", JSON.stringify(centerInfo, null, 2));

    // What's visible on the agent terminal?
    await window.screenshot({ path: resolve(outDir, "df-01-agent-tab.png") });

    // Check if there's a commit-approval UI anywhere visible.
    const approvalSniff = await window.evaluate(() => {
      const matches = Array.from(document.querySelectorAll("*"))
        .filter((el) => {
          const t = (el as HTMLElement).innerText ?? "";
          return /commit|approve|pending|propose/i.test(t) && t.length < 200;
        })
        .slice(0, 10)
        .map((el) => ({ tag: el.tagName, text: (el as HTMLElement).innerText.trim().slice(0, 120) }));
      return matches;
    });
    log("commit/approve UI hints:", JSON.stringify(approvalSniff, null, 2));

    // Click into the terminal area to focus it, then dump rows again.
    const xtermHost = window.locator(".xterm").first();
    if (await xtermHost.isVisible().catch(() => false)) {
      await xtermHost.click();
      await window.waitForTimeout(400);
      const rows2 = await window.evaluate(() => {
        const rows = document.querySelector(".xterm-rows");
        return rows ? (rows as HTMLElement).innerText : null;
      });
      writeFileSync(resolve(outDir, "df-terminal-rows.txt"), rows2 ?? "(none)");
      log("terminal rows dumped to df-terminal-rows.txt, first 400 chars:", (rows2 ?? "").slice(0, 400));
    } else {
      log("xterm not visible");
    }
  } finally {
    await close();
  }
}

main().catch((err) => {
  console.error("[dogfood] failed:", err);
  process.exit(1);
});
