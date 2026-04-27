// Dogfood pass (exploratory): poke the Hook Events page.
// Prior passes flagged xterm-noise from PreToolUse/PostToolUse as a
// `[F]`; this pass investigates whether the hook-events page
// already shows the same data more cleanly, which would justify
// suppressing the duplicate xterm output.
//
// Procedure:
//   1. Launch oxplow
//   2. Open the Hook events page (rail HUD → Pages → Hook events)
//   3. Prompt the inner agent with a trivial echo task so events flow
//   4. Capture the hook-events page rows + xterm rows side-by-side
//   5. The "finding" is the comparison itself — no commit expected
//      this pass unless something obviously broken surfaces.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { launchOxplow, probeLog, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchOxplow(projectDir);
  try {
    await window.waitForTimeout(3_000);
    probeLog("[hook-poke] launched");

    // Confirm the rail-HUD "Pages" entries are reachable
    const railPages = await window.evaluate(() =>
      Array.from(document.querySelectorAll("[data-testid^='rail-page-']"))
        .map((el) => el.getAttribute("data-testid"))
    );
    probeLog(`[hook-poke] rail pages: ${railPages.join(",")}`);

    // Open the Hook events page from the rail
    await window.getByTestId("rail-page-hook-events").click();
    await window.waitForTimeout(800);
    await window.screenshot({ path: resolve(outDir, "hook-poke-01-empty.png") });

    const initialPanelText = await window.evaluate(() =>
      (document.querySelector("[data-testid='page-hook-events']") as HTMLElement)?.innerText ?? "(no panel)"
    );
    probeLog(`[hook-poke] initial hook-events page (first 300): ${initialPanelText.slice(0, 300).replace(/\n/g, " ⏎ ")}`);

    // Now prompt the inner agent to do something tiny.
    const xterm = window.locator(".xterm").first();
    await xterm.waitFor({ state: "visible", timeout: 5_000 });
    await xterm.click();
    await window.waitForTimeout(300);
    await window.keyboard.type("Please run \`echo hello-from-hook-events-probe\` via the Bash tool. Do not propose a commit. Do not edit any files. Just run the one command.");
    await window.waitForTimeout(400);
    await window.keyboard.press("Enter");
    probeLog("[hook-poke] prompt sent");

    // Watch a few ticks
    for (let tick = 1; tick <= 10; tick++) {
      await window.waitForTimeout(15_000);
      const panelText = await window.evaluate(() =>
        (document.querySelector("[data-testid='page-hook-events']") as HTMLElement)?.innerText ?? "(no panel)"
      );
      const xtermText = await window.evaluate(() => {
        const r = document.querySelector(".xterm-rows");
        return r ? (r as HTMLElement).innerText : "";
      });
      writeFileSync(resolve(outDir, `hook-poke-panel-${String(tick).padStart(2, "0")}.txt`), panelText);
      writeFileSync(resolve(outDir, `hook-poke-xterm-${String(tick).padStart(2, "0")}.txt`), xtermText);
      await window.screenshot({ path: resolve(outDir, `hook-poke-poll-${String(tick).padStart(2, "0")}.png`) });
      probeLog(`[hook-poke] tick=${tick} panelLines=${panelText.split("\n").length} xtermLines=${xtermText.split("\n").length}`);
      if (panelText.includes("echo hello-from-hook-events-probe") || xtermText.includes("hello-from-hook-events-probe")) {
        probeLog(`[hook-poke] echo target seen on tick=${tick}; stopping early`);
        break;
      }
    }
  } finally {
    await close();
  }
}

runProbe("dogfood-hook-events-poke", main, { wallMs: 4 * 60_000, silenceMs: 90_000 }).catch(() => process.exit(1));
