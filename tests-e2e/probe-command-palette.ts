/**
 * Probe: Cmd/Ctrl+K command palette opens, filters, runs a command.
 *
 * Also verifies the palette shortcut fires when Monaco is focused — the
 * comment in `src/ui/App.tsx` claims a capture-phase listener wins over
 * Monaco's own keydown handler. This probe should catch regressions if
 * someone drops the `capture: true` flag.
 */
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchNewde, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchNewde(projectDir);
  try {
    await window.waitForTimeout(3_000);

    const isMac = process.platform === "darwin";
    const modKey = isMac ? "Meta" : "Control";

    // --- Part 1: palette opens from the default focus state ---
    await window.keyboard.press(`${modKey}+k`);
    await window.waitForTimeout(300);

    const paletteInput = window.getByTestId("command-palette-input");
    await paletteInput.waitFor({ timeout: 3_000 });
    await window.screenshot({ path: resolve(outDir, "palette-01-open.png") });

    // Fuzzy-typing "work new" should match "Work / New work item".
    await paletteInput.fill("work new");
    await window.waitForTimeout(150);

    const rowCount = await window.evaluate(() => {
      return document.querySelectorAll("[data-palette-row]").length;
    });
    console.log("[probe] palette rows after 'work new':", rowCount);
    if (rowCount === 0) {
      console.log("[probe] FAIL: fuzzy match 'work new' yielded no rows");
      process.exit(2);
    }

    // Verify newly-populated entries show up. "history" should match
    // "View / History" — regresses if history.open gets dropped.
    await paletteInput.fill("history");
    await window.waitForTimeout(150);
    const historyRows = await window.evaluate(() => {
      return Array.from(document.querySelectorAll("[data-palette-row]")).map((el) => el.textContent ?? "");
    });
    console.log("[probe] palette rows after 'history':", historyRows);
    if (!historyRows.some((t) => t.toLowerCase().includes("history"))) {
      console.log("[probe] FAIL: history.open not found in palette");
      process.exit(5);
    }

    // Dismiss via Escape.
    await paletteInput.fill("");
    await window.keyboard.press("Escape");
    await window.waitForTimeout(200);
    const stillOpen = await window.evaluate(() => !!document.querySelector('[data-testid="command-palette-input"]'));
    if (stillOpen) {
      console.log("[probe] FAIL: palette did not close on Escape");
      process.exit(3);
    }

    // --- Part 2: palette opens while Monaco is focused ---
    // Try to find and click a file in the file tree, wait for editor, then
    // focus Monaco, then fire Cmd+K.
    // Open the Files dock tab first — default view is Plan, so file-tree
    // nodes may be in the DOM but not visible.
    const filesDockTab = window.getByTestId("dock-tab-project");
    if (await filesDockTab.count() > 0) {
      await filesDockTab.click();
      await window.waitForTimeout(300);
    }

    const firstFile = window.locator('[data-testid^="file-tree-entry-"][data-kind="file"]:visible').first();
    const fileExists = await firstFile.count();
    if (fileExists > 0) {
      await firstFile.click();
      await window.waitForTimeout(800);

      // Focus Monaco's textarea if it's present.
      const monacoFocused = await window.evaluate(() => {
        const ta = document.querySelector<HTMLTextAreaElement>('.monaco-editor textarea');
        if (!ta) return false;
        ta.focus();
        return document.activeElement === ta;
      });
      console.log("[probe] monaco textarea focused:", monacoFocused);

      if (monacoFocused) {
        await window.keyboard.press(`${modKey}+k`);
        await window.waitForTimeout(300);
        const paletteFromMonaco = await window.evaluate(() => !!document.querySelector('[data-testid="command-palette-input"]'));
        await window.screenshot({ path: resolve(outDir, "palette-02-from-monaco.png") });
        if (!paletteFromMonaco) {
          console.log("[probe] FAIL: Cmd+K from Monaco did not open palette (capture-phase listener may be broken)");
          process.exit(4);
        }
        await window.keyboard.press("Escape");
        await window.waitForTimeout(200);
      } else {
        console.log("[probe] SKIP Monaco check: no textarea found");
      }
    } else {
      console.log("[probe] SKIP Monaco check: no files in tree");
    }

    console.log("[probe] OK: command palette opens, filters, closes, and survives Monaco focus");
  } finally {
    await close();
  }
}

runProbe("probe-command-palette", main).catch(() => process.exit(1));
