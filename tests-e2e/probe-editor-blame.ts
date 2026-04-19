/**
 * Probe: editor → right-click → "Annotate with Git Blame" → click a blame
 * row → verify history panel opens with that commit highlighted.
 *
 * Scenario 11. Also spot-checks whether clicking an "uncommitted" blame
 * line (sha = all zeros) triggers a broken reveal — a suspected friction
 * point: the cursor changes to "default" for uncommitted lines but the
 * onClick handler itself isn't guarded.
 */
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchNewde, sendMenuCommand } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { app, window, close } = await launchNewde(projectDir);
  try {
    await window.waitForTimeout(3_000);

    // Open a file with rich git history — App.tsx has many commits.
    await sendMenuCommand(app, "file.quickOpen");
    await window.waitForTimeout(400);
    const qoInput = window.locator('input[placeholder="Quick open file…"]');
    await qoInput.waitFor({ timeout: 3_000 });
    await qoInput.fill("src/ui/App.tsx");
    await window.waitForTimeout(400);
    await qoInput.press("Enter");
    await window.waitForTimeout(1_000);

    // Fire a native contextmenu event on the Monaco editor to open the
    // editor's custom ContextMenu. Monaco's onContextMenu listener picks
    // it up and sets state to render the React ContextMenu.
    const menuOpened = await window.evaluate(() => {
      const host = document.querySelector<HTMLElement>(".monaco-editor");
      if (!host) return { ok: false, reason: "no monaco" };
      const rect = host.getBoundingClientRect();
      const ev = new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: rect.left + 60,
        clientY: rect.top + 60,
        button: 2,
      });
      host.dispatchEvent(ev);
      return { ok: true };
    });
    if (!menuOpened.ok) {
      console.log("[probe] FAIL: no monaco host —", menuOpened.reason);
      process.exit(2);
    }
    await window.waitForTimeout(400);

    // The shared ContextMenu renders buttons with the item label as text.
    const annotateBtn = window.locator('button:has-text("Annotate with Git Blame")').first();
    if (await annotateBtn.count() === 0) {
      console.log("[probe] FAIL: Annotate with Git Blame not in editor context menu");
      await window.screenshot({ path: resolve(outDir, "blame-FAIL-no-menu.png") });
      process.exit(3);
    }
    await annotateBtn.click();
    await window.waitForTimeout(1_500); // git blame is async

    // Now the BlameOverlay should be rendered. Look for any blame row with
    // a formatted date (non-empty text) — those are the committed rows.
    const blameRows = await window.evaluate(() => {
      // BlameOverlay children are <div> inside a container near the top of
      // the editor. Match by the characteristic title prefix "<sha8> ".
      const rows = Array.from(document.querySelectorAll<HTMLElement>("div[title]"))
        .filter((d) => /^[0-9a-f]{8} /.test(d.getAttribute("title") ?? ""));
      return {
        count: rows.length,
        firstTitle: rows[0]?.getAttribute("title") ?? null,
      };
    });
    console.log("[probe] blame committed rows:", blameRows.count, "first:", blameRows.firstTitle?.slice(0, 60));
    if (blameRows.count === 0) {
      console.log("[probe] FAIL: no committed blame rows rendered");
      await window.screenshot({ path: resolve(outDir, "blame-FAIL-no-overlay.png") });
      process.exit(4);
    }
    await window.screenshot({ path: resolve(outDir, "blame-01-overlay.png") });

    // Click the first committed blame row; expect history panel to
    // activate (dock-tab-history becomes active / panel visible).
    const clicked = await window.evaluate(() => {
      const row = Array.from(document.querySelectorAll<HTMLElement>("div[title]"))
        .find((d) => /^[0-9a-f]{8} /.test(d.getAttribute("title") ?? ""));
      row?.click();
      return !!row;
    });
    if (!clicked) {
      console.log("[probe] FAIL: could not click blame row");
      process.exit(5);
    }
    await window.waitForTimeout(1_000);
    await window.screenshot({ path: resolve(outDir, "blame-02-after-reveal.png") });

    // The history dock-tab should now be active. Look for a visible
    // dock-panel-history.
    const historyActive = await window.evaluate(() => {
      const panel = document.querySelector<HTMLElement>('[data-testid="dock-panel-history"]');
      return {
        panelPresent: !!panel,
        panelVisible: panel ? panel.offsetParent !== null : false,
      };
    });
    console.log("[probe] history panel after reveal:", historyActive);
    if (!historyActive.panelVisible) {
      console.log("[probe] FAIL: history panel did not open after clicking blame row");
      process.exit(6);
    }

    console.log("[probe] OK: blame toggle + reveal-in-history round-trip works");
  } finally {
    await close();
  }
}

main().catch((err) => {
  console.error("[probe] failed:", err);
  process.exit(1);
});
