/**
 * Probe: Cmd/Ctrl+P quick-open — filter by substring, pick, opens file tab.
 *
 * Scenario 10. Also records friction around the substring match: e.g.
 * typing "app.tsx" matches "src/ui/App.tsx" (case-insensitive substring)
 * but typing non-contiguous ("apptsx") does NOT. That's the existing
 * QuickOpenOverlay design; this probe locks it in and leaves a follow-up.
 */
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchNewde, sendMenuCommand, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { app, window, close } = await launchNewde(projectDir);
  try {
    await window.waitForTimeout(3_000);

    // Open quick-open via menu-command IPC (keyboard would require Playwright
    // to drive the native macOS menu, which it can't).
    await sendMenuCommand(app, "file.quickOpen");
    await window.waitForTimeout(400);

    const qoInput = window.locator('input[placeholder="Quick open file…"]');
    await qoInput.waitFor({ timeout: 3_000 });
    await window.screenshot({ path: resolve(outDir, "quickopen-01-open.png") });

    // Substring filter — "App.tsx" should appear.
    await qoInput.fill("app.tsx");
    await window.waitForTimeout(400);

    const substringHits = await window.evaluate(() => {
      return Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
        .map((b) => b.textContent ?? "")
        .filter((t) => t.toLowerCase().includes("app.tsx"));
    });
    console.log("[probe] substring hits for 'app.tsx':", substringHits.length);
    if (substringHits.length === 0) {
      console.log("[probe] FAIL: substring match 'app.tsx' returned no rows");
      process.exit(2);
    }

    // Non-contiguous fuzzy — since quick-open shares `fuzzyMatches` with
    // the command palette, "apptsx" must match "app.tsx" (subsequence).
    await qoInput.fill("apptsx");
    await window.waitForTimeout(400);
    const fuzzyHits = await window.evaluate(() => {
      return Array.from(document.querySelectorAll<HTMLButtonElement>("button"))
        .map((b) => b.textContent ?? "")
        .filter((t) => t.toLowerCase().includes("app.tsx"));
    });
    console.log("[probe] non-contiguous 'apptsx' hits:", fuzzyHits.length);
    if (fuzzyHits.length === 0) {
      console.log("[probe] FAIL: quick-open should fuzzy-match 'apptsx' to App.tsx");
      process.exit(3);
    }

    // Pick via arrow + Enter.
    await qoInput.fill("app.tsx");
    await window.waitForTimeout(300);
    await qoInput.press("Enter");
    await window.waitForTimeout(600);

    // Overlay should close and a file: tab should be present.
    const overlayGone = await window.evaluate(() => !document.querySelector('input[placeholder="Quick open file…"]'));
    if (!overlayGone) {
      console.log("[probe] FAIL: quick-open did not close after Enter");
      process.exit(3);
    }
    const tabs = await window.evaluate(() => {
      return Array.from(document.querySelectorAll<HTMLElement>('[data-testid^="center-tab-file:"]'))
        .map((el) => el.dataset.testid);
    });
    console.log("[probe] file tabs after pick:", tabs);
    if (tabs.length === 0) {
      console.log("[probe] FAIL: no file tab opened after Enter");
      process.exit(4);
    }
    await window.screenshot({ path: resolve(outDir, "quickopen-02-picked.png") });

    console.log("[probe] OK: quick-open filters by substring, Enter opens the file tab");
  } finally {
    await close();
  }
}

runProbe("probe-quick-open", main).catch(() => process.exit(1));
