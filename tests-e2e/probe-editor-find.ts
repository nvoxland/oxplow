/**
 * Probe: Cmd+F (edit.find) opens Monaco's find widget on the active editor.
 *
 * Scenario from todo: "drive Monaco's find widget end-to-end and confirm no
 * shortcut collisions with app-level keybindings." We open a file, trigger
 * the menu command, then assert that Monaco's `.find-widget` becomes
 * visible (Monaco class-marks the widget with `visible` when open).
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

    // Open a file so there IS an editor to drive find on.
    await window.getByTestId("dock-tab-project").click();
    await window.waitForTimeout(300);
    const firstFile = window.locator('[data-testid^="file-tree-entry-"][data-kind="file"]:visible').first();
    await firstFile.waitFor({ timeout: 5_000 });
    await firstFile.click();
    await window.waitForTimeout(800);

    // Before: no find widget (Monaco lazily renders it on first invocation).
    const findBefore = await window.evaluate(() => {
      const w = document.querySelector(".monaco-editor .find-widget");
      return { present: !!w, visible: w?.classList.contains("visible") ?? false };
    });
    console.log("[probe] find widget before:", findBefore);

    // Trigger Cmd+F through the menu-command IPC.
    await sendMenuCommand(app, "edit.find");
    await window.waitForTimeout(500);

    const findAfter = await window.evaluate(() => {
      const w = document.querySelector(".monaco-editor .find-widget");
      return { present: !!w, visible: w?.classList.contains("visible") ?? false };
    });
    console.log("[probe] find widget after:", findAfter);
    await window.screenshot({ path: resolve(outDir, "editor-find-01-open.png") });

    if (!findAfter.present || !findAfter.visible) {
      console.log("[probe] FAIL: find widget did not become visible after edit.find");
      process.exit(2);
    }

    // Type into the find input and verify it accepts characters (i.e. the
    // app-level keybindings aren't hijacking typing inside the widget).
    // Monaco's find input is a textarea inside .find-widget (monaco uses
    // textareas for its inputs, not <input>).
    const findInput = window.locator(".monaco-editor .find-widget textarea").first();
    await findInput.waitFor({ timeout: 3_000 });
    await findInput.fill("import");
    await window.waitForTimeout(300);
    const findValue = await findInput.inputValue();
    if (findValue !== "import") {
      console.log("[probe] FAIL: find input dropped characters, got:", findValue);
      process.exit(3);
    }

    // Escape should close the widget.
    await findInput.press("Escape");
    await window.waitForTimeout(300);
    const findAfterEscape = await window.evaluate(() => {
      const w = document.querySelector(".monaco-editor .find-widget");
      return { visible: w?.classList.contains("visible") ?? false };
    });
    if (findAfterEscape.visible) {
      console.log("[probe] FAIL: find widget still visible after Escape");
      process.exit(4);
    }

    console.log("[probe] OK: edit.find opens Monaco find widget, accepts typing, closes on Escape");
  } finally {
    await close();
  }
}

runProbe("probe-editor-find", main).catch(() => process.exit(1));
